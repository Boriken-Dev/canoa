import 'dart:async';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart' show VoiceSettings;
import '../voice/speech_backend.dart';

/// Where a dictation is.
enum DictationState {
  idle,

  /// The microphone is open and the service is hearing.
  listening,

  /// The microphone is closed and the service is finishing the text.
  transcribing,

  /// The capture ended without text and [Dictation.error] says why.
  error,
}

/// Turns speech into text in the message being written. It records only between [start] and [stop] or [cancel], keeps nothing, and
/// never sends: the text it hears lands in the message at the cursor for the person to edit and send.
final class Dictation extends ChangeNotifier {
  Dictation({required this.backend, required this.voice, this.onFrontMatter});

  /// The speech service `[voice] backend` names, or null when this build has none of that name.
  final SpeechBackend? Function() backend;

  /// The person's choices for it: model, language and device.
  final VoiceSettings Function() voice;

  /// Told what the service said besides the words, when a dictation put text in the message and the service had something to say;
  /// the message then carries it to the agent. Returns why it could not, in words, or null.
  final String? Function(Map<String, Object?> frontMatter)? onFrontMatter;

  TextEditingController? _target;
  Future<bool> Function(SpeechBackend backend)? _confirm;

  DictationState _state = DictationState.idle;
  String? _error;
  String _provisional = '';
  int _seconds = 0;
  Timer? _clock;
  StreamSubscription<Transcript>? _heard;
  SpeechBackend? _using;
  final _finals = <String>[];
  final _notes = <String, Object?>{};
  Object? _failure;
  bool _cancelled = false;
  Future<void> _over = Future.value();
  final _told = <String>{};

  DictationState get state => _state;

  /// What went wrong, in words, while [state] is [DictationState.error].
  String? get error => _error;

  /// What the service heard so far and may still change; shown apart from the message and never put in it.
  String get provisional => _provisional;

  /// Whole seconds since the microphone opened.
  int get seconds => _seconds;

  /// Whether the microphone is open or the service is finishing; a new dictation waits for it to end.
  bool get busy => _state == DictationState.listening || _state == DictationState.transcribing;

  /// Makes [controller] the message dictation writes into. [confirm] says where speech goes before the first dictation with a
  /// service and answers whether to go on.
  void attach(TextEditingController controller, {Future<bool> Function(SpeechBackend backend)? confirm}) {
    _target = controller;
    _confirm = confirm;
  }

  /// Lets go of [controller]; a dictation into it ends and its text is dropped.
  void detach(TextEditingController controller) {
    if (!identical(_target, controller)) return;
    if (busy) unawaited(cancel(quiet: true));
    _target = null;
    _confirm = null;
  }

  /// Starts when idle, ends and keeps the text when listening.
  Future<void> toggle() => _state == DictationState.listening ? stop() : start();

  Future<void> start() async {
    if (busy) return;
    final target = _target;
    if (target == null) return;
    await _over;
    final service = backend();
    if (service == null) {
      _fail('No speech service is chosen. Choose one in Settings, under Voice.');
      return;
    }
    if (_told.add(service.id)) {
      final go = await (_confirm?.call(service) ?? Future.value(true));
      if (!go) {
        _told.remove(service.id);
        return;
      }
    }
    if (busy || !identical(_target, target)) return;
    _finals.clear();
    _notes.clear();
    _provisional = '';
    _failure = null;
    _error = null;
    _cancelled = false;
    _seconds = 0;
    _using = service;
    _state = DictationState.listening;
    final over = Completer<void>();
    _over = over.future;
    _clock = Timer.periodic(const Duration(seconds: 1), (_) {
      _seconds++;
      notifyListeners();
    });
    notifyListeners();
    final settings = voice();
    try {
      _heard = service.start(device: settings.device, language: settings.language, model: settings.model, profile: settings.profile).listen(
        (heard) {
          if (_cancelled) return;
          if (heard.isFinal) {
            if (heard.text.trim().isNotEmpty) _finals.add(heard.text.trim());
            _notes.addAll(heard.frontMatter);
            _provisional = '';
          } else {
            _provisional = heard.text;
          }
          notifyListeners();
        },
        onError: (Object error) => _failure ??= error,
        onDone: () => _finish(target, over),
      );
    } on Object catch (error) {
      _failure = error;
      _finish(target, over);
    }
  }

  /// Ends the capture and lets the service finish what it heard; its text goes into the message.
  Future<void> stop() async {
    if (_state != DictationState.listening) return;
    _stopClock();
    _state = DictationState.transcribing;
    notifyListeners();
    try {
      await _using?.stop();
    } on Object catch (error) {
      _failure ??= error;
    }
  }

  /// Ends the capture and puts nothing in the message.
  Future<void> cancel({bool quiet = false}) async {
    if (!busy) return;
    _cancelled = true;
    _stopClock();
    _provisional = '';
    _state = DictationState.idle;
    if (!quiet) notifyListeners();
    try {
      await _using?.cancel();
    } on Object {
      // The capture is ending anyway.
    }
  }

  /// Puts the error away.
  void dismiss() {
    if (_state != DictationState.error) return;
    _error = null;
    _state = DictationState.idle;
    notifyListeners();
  }

  void _finish(TextEditingController target, Completer<void> over) {
    _stopClock();
    _heard = null;
    _using = null;
    final failure = _failure;
    _failure = null;
    final text = _finals.join(' ');
    final notes = Map<String, Object?>.of(_notes);
    _notes.clear();
    _finals.clear();
    _provisional = '';
    final dropped = _cancelled || !identical(_target, target);
    if (dropped) {
      _state = DictationState.idle;
    } else if (failure != null) {
      _fail(failure is SpeechFailure ? failure.message : 'Dictation stopped: $failure');
    } else {
      _state = DictationState.idle;
      _insert(target, text);
      if (text.isNotEmpty && notes.isNotEmpty) {
        final why = onFrontMatter?.call(notes);
        if (why != null) _fail('The text is in your message, but what the service noted about it was not attached: $why');
      }
    }
    _cancelled = false;
    notifyListeners();
    if (!over.isCompleted) over.complete();
  }

  void _fail(String message) {
    _state = DictationState.error;
    _error = message;
    notifyListeners();
  }

  void _stopClock() {
    _clock?.cancel();
    _clock = null;
  }

  /// Puts [text] where the cursor is, or replaces what is selected, with a space on either side that needs one.
  void _insert(TextEditingController target, String text) {
    if (text.isEmpty) return;
    final value = target.value;
    final at = value.selection.isValid ? value.selection : TextSelection.collapsed(offset: value.text.length);
    final before = value.text.substring(0, at.start);
    final after = value.text.substring(at.end);
    final lead = before.isNotEmpty && !RegExp(r'\s$').hasMatch(before) ? ' ' : '';
    final trail = after.isNotEmpty && !RegExp(r'^\s').hasMatch(after) ? ' ' : '';
    target.value = TextEditingValue(
      text: '$before$lead$text$trail$after',
      selection: TextSelection.collapsed(offset: before.length + lead.length + text.length + trail.length),
    );
  }

  bool _disposed = false;

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _stopClock();
    unawaited(_heard?.cancel());
    super.dispose();
  }
}

/// The microphone beside the composer's controls: press to dictate, press again to stop. While it listens it shows the time, what
/// the service has heard so far and Stop and Cancel; when a dictation fails it says why in words.
class DictationButton extends StatefulWidget {
  const DictationButton({super.key, required this.dictation, required this.controller, required this.shortcut});

  final Dictation dictation;

  /// The message the text goes into.
  final TextEditingController controller;

  /// The keys that toggle it, for the tooltip.
  final String shortcut;

  @override
  State<DictationButton> createState() => _DictationButtonState();
}

class _DictationButtonState extends State<DictationButton> {
  @override
  void initState() {
    super.initState();
    widget.dictation.attach(widget.controller, confirm: _confirm);
  }

  @override
  void didUpdateWidget(DictationButton old) {
    super.didUpdateWidget(old);
    if (!identical(old.controller, widget.controller) || !identical(old.dictation, widget.dictation)) {
      old.dictation.detach(old.controller);
      widget.dictation.attach(widget.controller, confirm: _confirm);
    }
  }

  @override
  void dispose() {
    widget.dictation.detach(widget.controller);
    super.dispose();
  }

  /// Says where the audio goes before the first dictation with a service.
  Future<bool> _confirm(SpeechBackend backend) async {
    if (!mounted) return false;
    final go = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Dictate with ${backend.name}'),
        content: Text(
          '${backend.where.words}\n\nThe microphone is on only while you dictate, nothing is recorded to disk, and the text goes into '
          'your message for you to edit: it is never sent for you. You can change the service in Settings, under Voice.',
        ),
        actions: [
          TextButton(key: const Key('dictate-decline'), onPressed: () => Navigator.pop(context, false), child: const Text('Not now')),
          FilledButton(key: const Key('dictate-accept'), onPressed: () => Navigator.pop(context, true), child: const Text('Start dictating')),
        ],
      ),
    );
    return go ?? false;
  }

  String _clock(int seconds) => '${seconds ~/ 60}:${(seconds % 60).toString().padLeft(2, '0')}';

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.dictation,
    builder: (context, _) {
      final d = widget.dictation;
      final scheme = Theme.of(context).colorScheme;
      final mic = IconButton(
        key: const Key('dictate'),
        tooltip: 'Dictate (${widget.shortcut})',
        icon: const Icon(Icons.mic_none, size: 20),
        visualDensity: VisualDensity.compact,
        onPressed: () => unawaited(d.start()),
      );
      switch (d.state) {
        case DictationState.idle:
          return mic;
        case DictationState.listening:
          return Row(mainAxisSize: MainAxisSize.min, children: [
            IconButton(
              key: const Key('dictate-stop'),
              tooltip: 'Stop and put the text in the message',
              icon: Icon(Icons.stop_circle_outlined, size: 20, color: scheme.error),
              visualDensity: VisualDensity.compact,
              onPressed: () => unawaited(d.stop()),
            ),
            Text(_clock(d.seconds), key: const Key('dictate-time')),
            if (d.provisional.isNotEmpty)
              ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 180),
                child: Padding(
                  padding: const EdgeInsets.only(left: 6),
                  child: Text(d.provisional, key: const Key('dictate-heard'), maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(color: scheme.onSurfaceVariant, fontStyle: FontStyle.italic)),
                ),
              ),
            IconButton(
              key: const Key('dictate-cancel'),
              tooltip: 'Cancel and discard',
              icon: const Icon(Icons.close, size: 18),
              visualDensity: VisualDensity.compact,
              onPressed: () => unawaited(d.cancel()),
            ),
          ]);
        case DictationState.transcribing:
          return const Row(mainAxisSize: MainAxisSize.min, children: [
            SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2)),
            SizedBox(width: 8),
            Text('Transcribing…', key: Key('dictate-transcribing')),
          ]);
        case DictationState.error:
          return Row(mainAxisSize: MainAxisSize.min, children: [
            Icon(Icons.error_outline, size: 18, color: scheme.error),
            ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 260),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 6),
                child: Text(d.error ?? '', key: const Key('dictate-error'), maxLines: 2, overflow: TextOverflow.ellipsis, style: TextStyle(color: scheme.error)),
              ),
            ),
            IconButton(
              key: const Key('dictate-dismiss'),
              tooltip: 'Dismiss',
              icon: const Icon(Icons.close, size: 18),
              visualDensity: VisualDensity.compact,
              onPressed: d.dismiss,
            ),
            mic,
          ]);
      }
    },
  );
}
