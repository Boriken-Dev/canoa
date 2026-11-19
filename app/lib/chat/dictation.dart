import 'dart:async';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart' show DictationMode, VoiceSettings;
import '../voice/speech_backend.dart';

/// Where a dictation is.
enum DictationState {
  idle,

  /// The press is being answered: the service is being looked for and, the first time, the person is being asked where the audio
  /// goes. Cancel ends it.
  starting,

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
  Dictation({
    required this.backend,
    required this.voice,
    this.onFrontMatter,
    this.acknowledge,
    this.ready,
    this.whyNone,
    this.readyWithin = const Duration(seconds: 20),
    this.finishWithin = const Duration(minutes: 4),
  });

  /// How long [ready] may take before the dictation ends with a reason: listing a host's providers takes a moment, not a minute.
  final Duration readyWithin;

  /// How long a service may take, after Stop, to hand over its text or its failure. A service that says nothing in that time is
  /// told to cancel and the dictation ends with that as its reason, so the composer never waits for ever.
  final Duration finishWithin;

  /// The speech service `[voice] backend` names, or null when this build has none of that name.
  final SpeechBackend? Function() backend;

  /// Waited for before [backend] is asked, where the services are listed by something that may not have been read yet (the host's
  /// providers on a page that has just opened).
  final Future<void> Function()? ready;

  /// Why [backend] has none, in words, when there is a reason better than "none is chosen"; null says no more than that.
  final String? Function()? whyNone;

  /// The person's choices for it: model, language and device.
  final VoiceSettings Function() voice;

  /// Told what the service said besides the words, when a dictation put text in the message and the service had something to say;
  /// the message then carries it to the agent. Returns why it could not, in words, or null.
  final String? Function(Map<String, Object?> frontMatter)? onFrontMatter;

  /// Called with a service's id once the person has accepted where its audio goes, so the notice can be kept (in canoa.cfg) and not
  /// shown again for that service, in this run or the next. The services already in [voice]'s `acknowledged` are not asked about.
  final void Function(String id)? acknowledge;

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
  bool _held = false;
  bool _starting = false;

  /// Which capture is current: a service's late word about one that was cancelled or timed out reaches nothing.
  int _run = 0;
  Timer? _finishTimer;
  void Function()? _abandon;

  DictationState get state => _state;

  /// What went wrong, in words, while [state] is [DictationState.error].
  String? get error => _error;

  /// What the service heard so far and may still change; shown apart from the message and never put in it.
  String get provisional => _provisional;

  /// Whole seconds since the microphone opened.
  int get seconds => _seconds;

  /// Whether the microphone is open or the service is finishing; a new dictation waits for it to end.
  bool get busy => _state == DictationState.listening || _state == DictationState.transcribing;

  /// Whether Cancel has something to end: a start being answered, the microphone open or a service finishing.
  bool get cancellable => busy || _state == DictationState.starting;

  /// Makes [controller] the message dictation writes into. [confirm] says where speech goes before the first dictation with a
  /// service and answers whether to go on.
  void attach(TextEditingController controller, {Future<bool> Function(SpeechBackend backend)? confirm}) {
    _target = controller;
    _confirm = confirm;
  }

  /// Lets go of [controller]; a dictation into it ends and its text is dropped.
  void detach(TextEditingController controller) {
    if (!identical(_target, controller)) return;
    if (cancellable) unawaited(cancel(quiet: true));
    _target = null;
    _confirm = null;
  }

  /// Starts when idle, ends and keeps the text when listening.
  Future<void> toggle() => _state == DictationState.listening ? stop() : start();

  /// A press of the microphone or its shortcut: starts or ends a dictation in click-to-toggle mode, starts one in push-to-talk mode
  /// (where [release] ends it). Key repeats while held do nothing.
  Future<void> press() {
    if (voice().mode == DictationMode.toggle) return toggle();
    if (_held || busy) return Future.value();
    _held = true;
    return start(hold: true);
  }

  /// The microphone or its shortcut let go: in push-to-talk mode ends the dictation and keeps the text; one still waiting on the
  /// first-use notice does not open the microphone afterwards.
  Future<void> release() {
    if (!_held) return Future.value();
    _held = false;
    return _state == DictationState.listening ? stop() : Future.value();
  }

  /// Whether the shortcut is down for a push-to-talk dictation, so a key-up handler knows to call [release].
  bool get holding => _held;

  Future<void> start({bool hold = false}) async {
    if (busy || _starting) return;
    final target = _target;
    if (target == null) return;
    _starting = true;
    final run = ++_run;
    _error = null;
    _state = DictationState.starting;
    notifyListeners();
    try {
      await _begin(target, hold, run);
    } finally {
      // A start the person cancelled is no longer the current one, and a later press may already have begun another.
      if (run == _run) {
        _starting = false;
        if (_state == DictationState.starting) {
          _state = DictationState.idle;
          notifyListeners();
        }
      }
    }
  }

  Future<void> _begin(TextEditingController target, bool hold, int run) async {
    // A capture the person cancelled or that timed out is let go of, not waited for.
    await _over.timeout(const Duration(seconds: 2), onTimeout: () {});
    var listed = true;
    try {
      await ready?.call().timeout(readyWithin);
    } on TimeoutException {
      listed = false;
    } on Object {
      // What it could not read is told below, as there being no service.
    }
    if (run != _run) return;
    if (!listed) {
      _fail('The host did not list its speech providers within ${readyWithin.inSeconds} seconds. Check that it is running, then try again.');
      return;
    }
    final service = backend();
    if (service == null) {
      _fail(whyNone?.call() ?? 'No speech service is chosen. Choose one in Settings, under Voice.');
      return;
    }
    if (!voice().acknowledged.contains(service.id) && _told.add(service.id)) {
      final go = await (_confirm?.call(service) ?? Future.value(true));
      if (!go) {
        _told.remove(service.id);
        return;
      }
      acknowledge?.call(service.id);
    }
    if (run != _run || busy || !identical(_target, target)) return;
    if (hold && !_held) return;
    if (!hold) _held = false;
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
    _abandon = () => _finish(target, over, run);
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
        onDone: () => _finish(target, over, run),
      );
    } on Object catch (error) {
      _failure = error;
      _finish(target, over, run);
    }
  }

  /// Ends the capture and lets the service finish what it heard; its text goes into the message.
  Future<void> stop() async {
    if (_state != DictationState.listening) return;
    _stopClock();
    _state = DictationState.transcribing;
    notifyListeners();
    final service = _using, run = _run;
    _finishTimer?.cancel();
    _finishTimer = Timer(finishWithin, () {
      if (run != _run || _state != DictationState.transcribing) return;
      _failure ??= SpeechFailure('${service?.name ?? 'The speech service'} did not answer within ${_wait(finishWithin)} after you stopped. Nothing was added to your message.');
      unawaited(_end(service));
    });
    try {
      await _using?.stop();
    } on Object catch (error) {
      _failure ??= error;
    }
  }

  /// Ends the capture and puts nothing in the message.
  Future<void> cancel({bool quiet = false}) async {
    if (!cancellable) return;
    final starting = _state == DictationState.starting;
    final service = _using;
    _cancelled = true;
    _stopClock();
    _finishTimer?.cancel();
    _provisional = '';
    _state = DictationState.idle;
    if (starting) {
      _run++;
      _starting = false;
    }
    if (!quiet) notifyListeners();
    if (!starting) await _end(service, drop: true);
  }

  /// Tells [service] to give up and lets go of its capture now, whether or not it ever closes its stream.
  Future<void> _end(SpeechBackend? service, {bool drop = false}) async {
    final abandon = _abandon;
    try {
      await service?.cancel().timeout(const Duration(seconds: 5));
    } on Object {
      // The capture is ending anyway.
    }
    if (drop) _cancelled = true;
    abandon?.call();
  }

  static String _wait(Duration time) => time.inSeconds >= 120 ? '${time.inMinutes} minutes' : '${time.inSeconds} seconds';

  /// Puts the error away.
  void dismiss() {
    if (_state != DictationState.error) return;
    _error = null;
    _state = DictationState.idle;
    notifyListeners();
  }

  void _finish(TextEditingController target, Completer<void> over, int run) {
    if (run != _run) {
      if (!over.isCompleted) over.complete();
      return;
    }
    _run++;
    _stopClock();
    _finishTimer?.cancel();
    _finishTimer = null;
    _abandon = null;
    unawaited(_heard?.cancel());
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
      if (text.isEmpty) {
        _fail('Nothing was heard, so nothing was added to your message. Check the microphone and the speech service in Settings, under Voice.');
      } else if (notes.isNotEmpty) {
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
    _finishTimer?.cancel();
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
  Widget build(BuildContext context) => Listener(
    // The press lands on the microphone and the release anywhere: the microphone is replaced by Stop while it listens, so the
    // release is read here, above both.
    onPointerDown: (event) {
      if (widget.dictation.voice().mode == DictationMode.hold && widget.dictation.state == DictationState.idle && _overMic(event.position)) {
        unawaited(widget.dictation.press());
      }
    },
    onPointerUp: (_) => unawaited(widget.dictation.release()),
    onPointerCancel: (_) => unawaited(widget.dictation.release()),
    child: _body(),
  );

  bool _overMic(Offset at) {
    final box = _micKey.currentContext?.findRenderObject();
    if (box is! RenderBox || !box.attached) return false;
    return (box.localToGlobal(Offset.zero) & box.size).contains(at);
  }

  final _micKey = GlobalKey();

  Widget _body() => ListenableBuilder(
    listenable: widget.dictation,
    builder: (context, _) {
      final d = widget.dictation;
      final scheme = Theme.of(context).colorScheme;
      final hold = widget.dictation.voice().mode == DictationMode.hold;
      final mic = SizedBox(key: _micKey, child: IconButton(
        key: const Key('dictate'),
        tooltip: hold ? 'Hold to dictate, release to stop (${widget.shortcut})' : 'Dictate (${widget.shortcut})',
        icon: const Icon(Icons.mic_none, size: 20),
        visualDensity: VisualDensity.compact,
        // Push to talk starts on the press, below; a click has nothing more to do.
        onPressed: hold ? () {} : () => unawaited(d.start()),
      ));
      switch (d.state) {
        case DictationState.idle:
          return mic;
        case DictationState.starting:
          return Row(mainAxisSize: MainAxisSize.min, children: [
            const SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2)),
            const SizedBox(width: 8),
            const Text('Starting…', key: Key('dictate-starting')),
            IconButton(
              key: const Key('dictate-cancel'),
              tooltip: 'Cancel',
              icon: const Icon(Icons.close, size: 18),
              visualDensity: VisualDensity.compact,
              onPressed: () => unawaited(d.cancel()),
            ),
          ]);
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
          return Row(mainAxisSize: MainAxisSize.min, children: [
            const SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2)),
            const SizedBox(width: 8),
            const Text('Transcribing…', key: Key('dictate-transcribing')),
            IconButton(
              key: const Key('dictate-cancel'),
              tooltip: 'Cancel and discard',
              icon: const Icon(Icons.close, size: 18),
              visualDensity: VisualDensity.compact,
              onPressed: () => unawaited(d.cancel()),
            ),
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
