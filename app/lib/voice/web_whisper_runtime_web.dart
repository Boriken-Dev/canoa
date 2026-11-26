import 'dart:async';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

import '../brand.dart';
import 'speech_backend.dart';
import 'web_whisper.dart';

/// The worker beside the page, which loads the WebAssembly `whisper/canoa_whisper.js` builds.
const workerScript = 'whisper_worker.js';

/// The page's Whisper, or null in a browser that has no workers or WebAssembly.
WhisperRuntime? create() => web.window.has('Worker') && web.window.has('WebAssembly') ? _WorkerWhisper() : null;

/// A request a worker has not answered.
final class _Waiting {
  _Waiting(this.done, this.progress);

  final Completer<JSObject> done;
  final void Function(String words)? progress;
}

final class _WorkerWhisper implements WhisperRuntime {
  web.Worker? _worker;
  var _next = 0;
  final _waiting = <int, _Waiting>{};
  List<SpeechChoice>? _models;
  List<String> _endpoints = const [];

  web.Worker _start() {
    final existing = _worker;
    if (existing != null) return existing;
    final url = Uri.parse(web.document.baseURI).resolve(workerScript).toString();
    final worker = web.Worker(url.toJS, web.WorkerOptions(type: 'module'));
    worker.addEventListener('message', ((web.MessageEvent event) => _heard(event.data! as JSObject)).toJS);
    worker.addEventListener('error', ((web.Event event) => _lost('The Whisper worker could not start.')).toJS);
    return _worker = worker;
  }

  /// Ends every request a worker that is gone cannot answer, and lets the next one start another.
  void _lost(String why) {
    _worker?.terminate();
    _worker = null;
    final all = [..._waiting.values];
    _waiting.clear();
    for (final each in all) {
      if (!each.done.isCompleted) each.done.completeError(SpeechFailure(why));
    }
  }

  void _heard(JSObject message) {
    final id = (message['id'] as JSNumber?)?.toDartInt;
    final waiting = id == null ? null : _waiting[id];
    if (waiting == null) return;
    final progress = message['progress'];
    if (progress != null) {
      waiting.progress?.call((progress as JSString).toDart);
      return;
    }
    _waiting.remove(id);
    if ((message['ok'] as JSBoolean?)?.toDart ?? false) {
      waiting.done.complete(message);
    } else {
      final error = message['error'];
      waiting.done.completeError(SpeechFailure(_said(error.isA<JSString>() ? (error as JSString).toDart : 'the worker failed')));
    }
  }

  /// What a worker's error is said as: a refused download names the addresses the host's page policy has to allow.
  String _said(String error) {
    final blocked = error.contains('Failed to fetch') || error.contains('NetworkError') || error.contains('Load failed');
    if (!blocked) return error;
    final where = _endpoints.isEmpty ? "the model's publisher (huggingface.co)" : _endpoints.join(' ');
    return 'The model could not be downloaded. The page may only reach its own host: add $where to `connect` under `[host.web]` in '
        '${Brand.configFile}, or check the network.';
  }

  Future<JSObject> _ask(JSObject message, {JSArray<JSAny?>? transfer, void Function(String words)? progress}) {
    final worker = _start();
    final id = _next++;
    message['id'] = id.toJS;
    final waiting = _Waiting(Completer<JSObject>(), progress);
    _waiting[id] = waiting;
    if (transfer == null) {
      worker.postMessage(message);
    } else {
      worker.postMessage(message, transfer);
    }
    return waiting.done.future;
  }

  @override
  Future<List<SpeechChoice>> models() async {
    final known = _models;
    if (known != null) return known;
    final answer = await _ask(JSObject()..['type'] = 'catalog'.toJS);
    final catalog = answer['catalog']! as JSObject;
    final listed = (catalog['models']! as JSArray<JSObject>).toDart;
    _endpoints = [for (final each in (catalog['endpoints']! as JSArray<JSString>).toDart) each.toDart];
    return _models = [for (final model in listed) SpeechChoice((model['id']! as JSString).toDart, (model['label']! as JSString).toDart)];
  }

  @override
  Future<WhisperHeard> transcribe({
    required String model,
    required Float32List samples,
    String? language,
    String? prompt,
    void Function(String words)? progress,
  }) async {
    await models();
    final message = JSObject()
      ..['type'] = 'transcribe'.toJS
      ..['model'] = model.toJS
      ..['samples'] = samples.toJS
      ..['language'] = (language ?? '').toJS
      ..['prompt'] = (prompt ?? '').toJS;
    final answer = await _ask(message, transfer: <JSAny?>[samples.buffer.toJS].toJS, progress: progress);
    final said = answer['language'];
    return WhisperHeard((answer['text']! as JSString).toDart, language: said.isA<JSString>() ? (said as JSString).toDart : null);
  }
}
