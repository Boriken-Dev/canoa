import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../config/canoa_config.dart';
import '../l10n/app_text.dart';
import '../settings/settings_controller.dart';
import '../settings/settings_row.dart';
import 'outputs.dart';
import 'reply_reader.dart';
import 'speech_output.dart';

/// The choice that clears a key, so the provider's own default applies.
const _own = '';

/// What the sample says, read in whatever voice is chosen.
const readAloudSample = 'This is how replies will sound. The voice, model and speed are the ones chosen here.';

/// The speeds offered, as multiples of normal speech, where the provider takes them.
const readAloudSpeeds = [0.75, 1.0, 1.25, 1.5, 2.0];

/// Settings > Voice, the reading half: who reads a reply aloud and with which voice, model, style and speed, where the text goes
/// while it is read, and whether each reply is read as it arrives. Each part is written to `[voice]` as it is changed.
class ReadAloudGroup extends StatefulWidget {
  const ReadAloudGroup({super.key, required this.controller, required this.outputs, this.reader});

  final SettingsController controller;
  final VoiceOutputs outputs;

  /// What reads the sample; without it there is no way to try a voice.
  final ReplyReader? reader;

  @override
  State<ReadAloudGroup> createState() => _ReadAloudGroupState();
}

class _ReadAloudGroupState extends State<ReadAloudGroup> {
  SpeechOutput? _listedFor;
  Future<List<OutputVoice>>? _voices;

  @override
  void initState() {
    super.initState();
    unawaited(widget.outputs.ready().then((_) {}, onError: (_) {}));
  }

  void _set(SpeakSettings Function(SpeakSettings) change) =>
      widget.controller.change((c) => c.copyWith(voice: c.voice.copyWith(speak: change(c.voice.speak))));

  Future<List<OutputVoice>> _voicesOf(SpeechOutput output) {
    if (!identical(output, _listedFor)) {
      _listedFor = output;
      _voices = output.voices().then((v) => v, onError: (_) => const <OutputVoice>[]);
    }
    return _voices!;
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([widget.controller, widget.outputs]),
    builder: (context, _) {
      final speak = widget.controller.config.voice.speak;
      final all = widget.outputs.all;
      if (all.isEmpty) {
        return SettingsGroup(title: 'Read replies aloud', children: [
          SettingsRow(
            key: const Key('speak-none'),
            title: 'No voice to read replies aloud with',
            description: widget.outputs.problem != null
                ? "This host's voice providers are not available: ${widget.outputs.problem}"
                : 'This browser or system has no voices of its own, and Canoa never sends text to another service on its own. '
                    'Add a voice provider on the host (canoa.cfg, under say), and replies are read through it',
            child: true,
          ),
        ]);
      }
      final output = widget.outputs.resolve(speak.output);
      return SettingsGroup(title: 'Read replies aloud', children: [
        ChoiceRow<String>(
          title: 'Voice provider',
          configKey: const ['voice', 'output'],
          description: 'Who turns a reply into speech',
          fieldKey: const Key('speak-output'),
          value: speak.output != null && output != null ? output.id : (speak.output ?? _own),
          choices: [
            (_own, 'Automatic'),
            for (final each in all) (each.id, each.name),
            if (speak.output != null && output == null) (speak.output!, '${speak.output} (not available here)'),
          ],
          onChanged: (id) => _set((s) => SpeakSettings(output: id == _own ? null : id, speed: s.speed, readReplies: s.readReplies)),
        ),
        if (output == null)
          SettingsRow(
            key: const Key('speak-missing'),
            title: 'Not available here',
            description: 'The provider named in canoa.cfg is not here. Choose another; nothing is spoken meanwhile',
            child: true,
          )
        else ..._choices(context, speak, output),
        SwitchRow(
          key: const Key('speak-replies'),
          title: 'Read replies as they arrive',
          configKey: const ['voice', 'read_replies'],
          description: 'Speaks each reply a sentence at a time while it streams in. Typing or dictating stops it, and so does the '
              "button beside the message box. Code blocks and a tool's output are not read",
          value: speak.readReplies,
          onChanged: (on) => _set((s) => s.copyWith(readReplies: on)),
        ),
      ]);
    },
  );

  List<Widget> _choices(BuildContext context, SpeakSettings speak, SpeechOutput output) {
    final models = output.models;
    return [
      SettingsRow(
        key: const Key('speak-where'),
        title: 'Where the text goes',
        description: switch (output.where.leaves) {
          TextLeaves.no => 'It stays on this device or your host. ${output.where.words}',
          TextLeaves.maybe => 'It may leave this device, as the provider decides. ${output.where.words}',
          TextLeaves.yes => 'It leaves this device for the provider. ${output.where.words}',
        },
        child: true,
      ),
      if (output.where.endpoint case final endpoint?)
        SettingsRow(key: const Key('speak-endpoint'), title: 'Account or address', description: endpoint, child: true),
      if (output.uses.contains('voice'))
        FutureBuilder<List<OutputVoice>>(
          future: _voicesOf(output),
          builder: (context, snapshot) {
            final voices = snapshot.data ?? const <OutputVoice>[];
            return ChoiceRow<String>(
              title: 'Voice',
              configKey: const ['voice', 'output_voice'],
              description: snapshot.connectionState != ConnectionState.done
                  ? 'Reading the voices'
                  : voices.isEmpty
                  ? (output.voicesProblem == null ? 'This provider lists no voices' : context.t('speak.voices_problem', {'why': output.voicesProblem}))
                  : null,
              fieldKey: const Key('speak-voice'),
              value: speak.voice ?? _own,
              choices: [
                (_own, 'Provider default'),
                for (final voice in voices) (voice.id, voice.language == null ? voice.name : '${voice.name} (${voice.language})'),
                if (speak.voice != null && !voices.any((v) => v.id == speak.voice)) (speak.voice!, speak.voice!),
              ],
              onChanged: (id) => _set((s) => id == _own ? s.copyWith(clearVoice: true) : s.copyWith(voice: id)),
            );
          },
        ),
      if (output.uses.contains('voice'))
        SettingsRow(
          key: const Key('speak-voice-name-row'),
          title: context.t('speak.voice_name'),
          description: context.t('speak.voice_name_help'),
          control: SizedBox(
            width: 200,
            child: _VoiceName(
              key: ValueKey('speak-voice-name-${output.id}'),
              value: speak.voice ?? '',
              onChanged: (name) => _set((s) => name.isEmpty ? s.copyWith(clearVoice: true) : s.copyWith(voice: name)),
            ),
          ),
        ),
      if (output.uses.contains('model') && models.isNotEmpty)
        ChoiceRow<String>(
          title: 'Model',
          configKey: const ['voice', 'output_model'],
          fieldKey: const Key('speak-model'),
          value: speak.model ?? _own,
          choices: [
            (_own, output.defaultModel == null ? 'Provider default' : 'Default (${_label(models, output.defaultModel!)})'),
            for (final model in models) (model.id, model.label),
            if (speak.model != null && !models.any((m) => m.id == speak.model)) (speak.model!, speak.model!),
          ],
          onChanged: (id) => _set((s) => id == _own ? s.copyWith(clearModel: true) : s.copyWith(model: id)),
        ),
      if (output.uses.contains('style') && output.styles.isNotEmpty)
        ChoiceRow<String>(
          title: 'Style',
          configKey: const ['voice', 'output_style'],
          fieldKey: const Key('speak-style'),
          value: speak.style ?? _own,
          choices: [
            (_own, 'Provider default'),
            for (final style in output.styles) (style.id, style.label),
            if (speak.style != null && !output.styles.any((s) => s.id == speak.style)) (speak.style!, speak.style!),
          ],
          onChanged: (id) => _set((s) => id == _own ? s.copyWith(clearStyle: true) : s.copyWith(style: id)),
        ),
      if (output.uses.contains('speed') && output.speed != null) _speed(speak, output.speed!),
      if (widget.reader case final reader?) _Try(reader: reader),
    ];
  }

  Widget _speed(SpeakSettings speak, SpeedRange range) {
    final offered = [for (final speed in readAloudSpeeds) if (speed >= range.min && speed <= range.max) speed];
    final held = speak.speed;
    return ChoiceRow<double>(
      title: 'Speed',
      configKey: const ['voice', 'output_speed'],
      fieldKey: const Key('speak-speed'),
      value: held == null || held == 1.0 ? 0 : held,
      choices: [
        (0, 'Normal'),
        for (final speed in offered) if (speed != 1.0) (speed, '${speed}x'),
        if (held != null && held != 1.0 && !offered.contains(held)) (held, '${held}x'),
      ],
      onChanged: (speed) => _set((s) => speed == 0 ? s.copyWith(clearSpeed: true) : s.copyWith(speed: speed)),
    );
  }

  static String _label(List<OutputChoice> choices, String id) => choices.where((c) => c.id == id).firstOrNull?.label ?? id;
}

/// A voice written by name, for one a provider does not list (a custom voice, or one that is newer than the list): sent as typed.
class _VoiceName extends StatefulWidget {
  const _VoiceName({super.key, required this.value, required this.onChanged});

  final String value;
  final ValueChanged<String> onChanged;

  @override
  State<_VoiceName> createState() => _VoiceNameState();
}

class _VoiceNameState extends State<_VoiceName> {
  late final TextEditingController _text = TextEditingController(text: widget.value);

  @override
  void didUpdateWidget(_VoiceName old) {
    super.didUpdateWidget(old);
    if (widget.value != old.value && widget.value != _text.text.trim()) _text.text = widget.value;
  }

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _commit() {
    final name = _text.text.trim();
    if (name != widget.value) widget.onChanged(name);
  }

  @override
  Widget build(BuildContext context) => Focus(
    onFocusChange: (focused) => focused ? null : _commit(),
    child: TextField(key: const Key('speak-voice-name'), controller: _text, onSubmitted: (_) => _commit()),
  );
}

/// Reads the sample, so a voice can be heard before a reply is.
class _Try extends StatelessWidget {
  const _Try({required this.reader});

  final ReplyReader reader;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: reader,
    builder: (context, _) {
      final reading = reader.isReading('sample');
      final failed = reader.errorKey == 'sample' ? reader.error : null;
      return SettingsRow(
        title: 'Try the voice',
        description: reading ? 'Reading a sample' : 'Reads a sentence with the choices above',
        control: reading
            ? FilledButton.tonal(key: const Key('speak-try-stop'), onPressed: reader.stop, child: const Text('Stop'))
            : OutlinedButton(
                key: const Key('speak-try'),
                onPressed: () {
                  reader.clearError();
                  unawaited(reader.toggle('sample', readAloudSample));
                },
                child: const Text('Play'),
              ),
        below: failed == null ? null : Text(failed, key: const Key('speak-try-error'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
      );
    },
  );
}
