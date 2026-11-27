// A second Flutter view drawn into a window.open popup's document, as a bare page for a browser test: what a view in another
// browser window can do (typing, composition, keys, focus, frames, close) with the engine's own handling, and which of the shims
// in `popup_view/shims.js` each gap needs. The app starts in multi-view mode (`popup_view/index.html` hands the page's own
// view to it and keeps the engine's `FlutterApp` as `window.flutterApp`); a click on "open" calls `window.open` with no await
// before it and adds a view to a host element of the popup's document. Built beside the app with
//   flutter build web --release --no-web-resources-cdn -t tool/popup_view_page.dart -o <dir>
// and driven by `clients/browser/chromium/e2e/popup_view.mjs`, which reads what the page holds through `probeState()`.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:web/web.dart' as web;

/// One probe view's widgets, kept by view id so `probeState()` reads them while they are alive.
final class _ViewProbe {
  final a = TextEditingController();
  final b = TextEditingController();
  final aFocus = FocusNode();
  final bFocus = FocusNode();
  final aKey = GlobalKey();
  final bKey = GlobalKey();
  final openKey = GlobalKey();
  final incKey = GlobalKey();
  double ratio = 0;
  double width = 0;
  int submitted = 0;
  int pings = 0;
}

final _probes = <int, _ViewProbe>{};
final shared = ValueNotifier<int>(0);
int _frames = 0;
final _keys = <String>[];
final _popups = <web.Window>[];
final _popupViews = <int>[];

JSObject get _globals => globalContext;

Map<String, Object?>? _rect(GlobalKey key) {
  final box = key.currentContext?.findRenderObject();
  if (box is! RenderBox || !box.attached) return null;
  final at = box.localToGlobal(Offset.zero);
  return {'x': at.dx, 'y': at.dy, 'w': box.size.width, 'h': box.size.height};
}

String _state() => jsonEncode({
  'frames': _frames,
  'shared': shared.value,
  'keys': _keys,
  'popups': _popups.where((popup) => !popup.closed).length,
  'popupViews': _popupViews,
  'views': {
    for (final entry in _probes.entries)
      '${entry.key}': {
        'a': entry.value.a.text,
        'selection': [entry.value.a.selection.start, entry.value.a.selection.end],
        'composing': [entry.value.a.value.composing.start, entry.value.a.value.composing.end],
        'b': entry.value.b.text,
        'aFocus': entry.value.aFocus.hasFocus,
        'bFocus': entry.value.bFocus.hasFocus,
        'submitted': entry.value.submitted,
        'pings': entry.value.pings,
        'ratio': entry.value.ratio,
        'width': entry.value.width,
        'rects': {
          'a': _rect(entry.value.aKey),
          'b': _rect(entry.value.bKey),
          'open': _rect(entry.value.openKey),
          'inc': _rect(entry.value.incKey),
        },
      },
  },
});

/// Opens the popup in the click's own task and adds a view of this same app to a host element of its document.
void _openPopup() {
  final popup = web.window.open('', 'probe-popup-${_popups.length}', 'popup=yes,width=640,height=520');
  if (popup == null) return;
  final document = popup.document;
  document.title = 'probe popup';
  document.documentElement!.setAttribute('style', 'height:100%;margin:0');
  document.body!.setAttribute('style', 'height:100%;margin:0;overflow:hidden');
  final host = document.createElement('div') as web.HTMLElement;
  host.setAttribute('style', 'position:absolute;inset:0');
  document.body!.append(host);
  final shims = _globals['probeShims'];
  if (shims.isA<JSObject>()) (shims as JSObject).callMethod('install'.toJS, popup);
  final app = _globals['flutterApp'] as JSObject;
  final options = JSObject()..['hostElement'] = host;
  final id = (app.callMethod('addView'.toJS, options) as JSNumber).toDartInt;
  _popups.add(popup);
  _popupViews.add(id);
  popup.addEventListener(
    'pagehide',
    ((web.Event _) {
      app.callMethod('removeView'.toJS, id.toJS);
      _popupViews.remove(id);
    }).toJS,
  );
}

/// Closing from the page: the views are removed, then the popups closed.
void _closePopups() {
  final app = _globals['flutterApp'] as JSObject;
  for (final id in [..._popupViews]) {
    app.callMethod('removeView'.toJS, id.toJS);
    _popupViews.remove(id);
  }
  for (final popup in _popups) {
    popup.close();
  }
}

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  HardwareKeyboard.instance.addHandler((event) {
    _keys.add('${event is KeyDownEvent ? 'down' : event is KeyUpEvent ? 'up' : 'repeat'}:${event.logicalKey.keyLabel}');
    if (_keys.length > 40) _keys.removeAt(0);
    return false;
  });
  SchedulerBinding.instance.addPersistentFrameCallback((_) => _frames++);
  _globals['probeState'] = (() => _state().toJS).toJS;
  _globals['probeReset'] = (() {
    _keys.clear();
    for (final probe in _probes.values) {
      probe.submitted = 0;
      probe.pings = 0;
    }
  }).toJS;
  _globals['probeClose'] = _closePopups.toJS;
  // The opener's pagehide takes the popups with it: their views die with this isolate.
  web.window.addEventListener('pagehide', ((web.Event _) => _closePopups()).toJS);
  runWidget(const _Views());
}

final class _Views extends StatefulWidget {
  const _Views();

  @override
  State<_Views> createState() => _ViewsState();
}

final class _ViewsState extends State<_Views> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeMetrics() => setState(() {});

  @override
  Widget build(BuildContext context) {
    final views = WidgetsBinding.instance.platformDispatcher.views;
    _probes.removeWhere((id, _) => !views.any((view) => view.viewId == id));
    return ViewCollection(views: [for (final view in views) View(view: view, child: _Probe(view.viewId))]);
  }
}

final class _PingIntent extends Intent {
  const _PingIntent();
}

final class _Probe extends StatefulWidget {
  const _Probe(this.id);

  final int id;

  @override
  State<_Probe> createState() => _ProbeState();
}

final class _ProbeState extends State<_Probe> with SingleTickerProviderStateMixin {
  late final _ViewProbe probe = _probes.putIfAbsent(widget.id, _ViewProbe.new);
  late final AnimationController spin = AnimationController(vsync: this, duration: const Duration(seconds: 1))..repeat();

  @override
  void dispose() {
    spin.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final media = MediaQuery.of(context);
    probe.ratio = media.devicePixelRatio;
    probe.width = media.size.width;
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      home: Scaffold(
        body: Shortcuts(
          shortcuts: const {SingleActivator(LogicalKeyboardKey.keyK, control: true): _PingIntent()},
          child: Actions(
            actions: {
              _PingIntent: CallbackAction<_PingIntent>(
                onInvoke: (_) {
                  probe.pings++;
                  return null;
                },
              ),
            },
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text('view ${widget.id}'),
                  AnimatedBuilder(animation: spin, builder: (_, _) => LinearProgressIndicator(value: spin.value)),
                  const SizedBox(height: 12),
                  TextField(
                    key: probe.aKey,
                    controller: probe.a,
                    focusNode: probe.aFocus,
                    textInputAction: TextInputAction.done,
                    // Done would take the focus away, and the checks after Enter need it kept.
                    onEditingComplete: () {},
                    onSubmitted: (_) => probe.submitted++,
                  ),
                  const SizedBox(height: 12),
                  TextField(key: probe.bKey, controller: probe.b, focusNode: probe.bFocus),
                  const SizedBox(height: 12),
                  ValueListenableBuilder<int>(valueListenable: shared, builder: (_, value, _) => Text('shared $value')),
                  Row(
                    children: [
                      ElevatedButton(key: probe.openKey, onPressed: _openPopup, child: const Text('open')),
                      const SizedBox(width: 12),
                      ElevatedButton(key: probe.incKey, onPressed: () => shared.value++, child: const Text('inc')),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
