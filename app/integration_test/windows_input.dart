// Input as Windows delivers a person's to this process's own Flutter view, in one of two ways:
//
// - `device`: SendInput, the keyboard's and the mouse's own way in, through everything Windows does with a keystroke (keyboard
//   hooks, text services, the window in front) before the engine sees it. It needs the window in front, so an unlocked desktop,
//   and it refuses to send while another window is in front.
// - `posted`: the key and mouse messages posted to the engine's window. They pass through the runner's message loop
//   (TranslateMessage makes each key's WM_CHAR), the engine's keyboard manager, its text input and its pointer handling, whether
//   or not the window is in front, and reach no other window; what Windows does before a message is queued is left out.
//
// Windows only.
import 'dart:ffi';
import 'dart:io';

final _user32 = DynamicLibrary.open('user32.dll');
final _kernel32 = DynamicLibrary.open('kernel32.dll');

final _findWindowEx = _user32.lookupFunction<IntPtr Function(IntPtr, IntPtr, Pointer<Uint16>, Pointer<Uint16>), int Function(int, int, Pointer<Uint16>, Pointer<Uint16>)>('FindWindowExW');
final _windowThreadProcessId = _user32.lookupFunction<Uint32 Function(IntPtr, Pointer<Uint32>), int Function(int, Pointer<Uint32>)>('GetWindowThreadProcessId');
final _isWindowVisible = _user32.lookupFunction<Int32 Function(IntPtr), int Function(int)>('IsWindowVisible');
final _postMessage = _user32.lookupFunction<Int32 Function(IntPtr, Uint32, IntPtr, IntPtr), int Function(int, int, int, int)>('PostMessageW');
final _foregroundWindow = _user32.lookupFunction<IntPtr Function(), int Function()>('GetForegroundWindow');
final _setForegroundWindow = _user32.lookupFunction<Int32 Function(IntPtr), int Function(int)>('SetForegroundWindow');
final _bringWindowToTop = _user32.lookupFunction<Int32 Function(IntPtr), int Function(int)>('BringWindowToTop');
final _attachThreadInput = _user32.lookupFunction<Int32 Function(Uint32, Uint32, Int32), int Function(int, int, int)>('AttachThreadInput');
final _openInputDesktop = _user32.lookupFunction<IntPtr Function(Uint32, Int32, Uint32), int Function(int, int, int)>('OpenInputDesktop');
final _closeDesktop = _user32.lookupFunction<Int32 Function(IntPtr), int Function(int)>('CloseDesktop');
final _clientToScreen = _user32.lookupFunction<Int32 Function(IntPtr, Pointer<_Point>), int Function(int, Pointer<_Point>)>('ClientToScreen');
final _systemMetrics = _user32.lookupFunction<Int32 Function(Int32), int Function(int)>('GetSystemMetrics');
final _sendInput = _user32.lookupFunction<Uint32 Function(Uint32, Pointer<_Input>, Int32), int Function(int, Pointer<_Input>, int)>('SendInput');
final _vkKeyScan = _user32.lookupFunction<Int16 Function(Uint16), int Function(int)>('VkKeyScanW');
final _mapVirtualKey = _user32.lookupFunction<Uint32 Function(Uint32, Uint32), int Function(int, int)>('MapVirtualKeyW');
final _currentThreadId = _kernel32.lookupFunction<Uint32 Function(), int Function()>('GetCurrentThreadId');
final _localAlloc = _kernel32.lookupFunction<Pointer<Void> Function(Uint32, IntPtr), Pointer<Void> Function(int, int)>('LocalAlloc');
final _localFree = _kernel32.lookupFunction<Pointer<Void> Function(Pointer<Void>), Pointer<Void> Function(Pointer<Void>)>('LocalFree');

final class _Point extends Struct {
  @Int32()
  external int x;
  @Int32()
  external int y;
}

final class _MouseInput extends Struct {
  @Int32()
  external int dx;
  @Int32()
  external int dy;
  @Uint32()
  external int mouseData;
  @Uint32()
  external int flags;
  @Uint32()
  external int time;
  @IntPtr()
  external int extraInfo;
}

final class _KeyboardInput extends Struct {
  @Uint16()
  external int vk;
  @Uint16()
  external int scan;
  @Uint32()
  external int flags;
  @Uint32()
  external int time;
  @IntPtr()
  external int extraInfo;
}

final class _Either extends Union {
  external _MouseInput mouse;
  external _KeyboardInput keyboard;
}

final class _Input extends Struct {
  @Uint32()
  external int type;
  external _Either input;
}

const _wmMouseMove = 0x0200, _wmLeftDown = 0x0201, _wmLeftUp = 0x0202, _wmKeyDown = 0x0100, _wmKeyUp = 0x0101, _mkLeft = 0x0001;
const _inputMouse = 0, _inputKeyboard = 1, _keyEventUp = 0x0002;
const _mouseMove = 0x0001, _mouseLeftDown = 0x0002, _mouseLeftUp = 0x0004, _mouseVirtualDesk = 0x4000, _mouseAbsolute = 0x8000;
const _vkShift = 0x10, _vkControl = 0x11, _vkMenu = 0x12;

/// Named keys [WindowsInput.key] presses.
const windowsKeys = {'enter': 0x0D, 'tab': 0x09, 'escape': 0x1B, 'backspace': 0x08};

/// How input reaches the window; see the top of this file.
enum WindowsInputMode { device, posted }

/// What could not be sent, and why; nothing past that point was.
final class WindowsInputRefused implements Exception {
  const WindowsInputRefused(this.message);

  final String message;

  @override
  String toString() => message;
}

/// Whether the desktop takes input now: false while the session is locked or another desktop (a UAC prompt) has it.
bool get inputDesktopOpen {
  final desktop = _openInputDesktop(0, 0, 0x0100); // DESKTOP_SWITCHDESKTOP
  if (desktop == 0) return false;
  _closeDesktop(desktop);
  return true;
}

/// The Flutter view of this process's window, and input to it.
final class WindowsInput {
  WindowsInput._(this.mode, this.window, this.view);

  /// The view inside this process's visible Flutter window.
  factory WindowsInput.ofThisProcess(WindowsInputMode mode) {
    final runnerClass = _wide('FLUTTER_RUNNER_WIN32_WINDOW');
    final viewClass = _wide('FLUTTERVIEW');
    final owner = _localAlloc(0x40, 4).cast<Uint32>();
    try {
      var at = 0;
      while (true) {
        at = _findWindowEx(0, at, runnerClass, nullptr);
        if (at == 0) throw const WindowsInputRefused('this process has no Flutter window');
        _windowThreadProcessId(at, owner);
        if (owner.value != pid || _isWindowVisible(at) == 0) continue;
        final view = _findWindowEx(at, 0, viewClass, nullptr);
        if (view == 0) throw const WindowsInputRefused('the Flutter window holds no view');
        return WindowsInput._(mode, at, view);
      }
    } finally {
      _localFree(runnerClass.cast());
      _localFree(viewClass.cast());
      _localFree(owner.cast());
    }
  }

  final WindowsInputMode mode;

  /// The top-level window, and the engine's window for the view inside it.
  final int window, view;

  bool get inFront => _foregroundWindow() == window;

  /// For [WindowsInputMode.device], brings the window in front, which Windows allows a process only while it shares the input of
  /// the window in front; throws when it is still not in front, so that nothing typed lands in another window.
  Future<void> ready() async {
    if (mode == WindowsInputMode.posted) return;
    if (!inputDesktopOpen) throw const WindowsInputRefused('the desktop takes no input (the session is locked): nothing was sent');
    if (inFront) return;
    final front = _foregroundWindow();
    final frontThread = front == 0 ? 0 : _windowThreadProcessId(front, nullptr);
    final mine = _currentThreadId();
    if (frontThread != 0 && frontThread != mine) _attachThreadInput(mine, frontThread, 1);
    _bringWindowToTop(window);
    _setForegroundWindow(window);
    if (frontThread != 0 && frontThread != mine) _attachThreadInput(mine, frontThread, 0);
    await Future<void>.delayed(const Duration(milliseconds: 300));
    if (!inFront) throw const WindowsInputRefused('the window could not be brought in front: nothing was sent');
  }

  void _post(int message, int wParam, int lParam) {
    if (_postMessage(view, message, wParam, lParam) == 0) throw WindowsInputRefused('the view refused message 0x${message.toRadixString(16)}');
  }

  void _send(List<void Function(Pointer<_Input>)> fill) {
    if (!inFront) throw const WindowsInputRefused('another window is in front: input stopped');
    final size = sizeOf<_Input>();
    final inputs = _localAlloc(0x40, size * fill.length).cast<_Input>();
    try {
      for (final (index, each) in fill.indexed) {
        each(inputs + index);
      }
      final sent = _sendInput(fill.length, inputs, size);
      if (sent != fill.length) throw WindowsInputRefused('SendInput sent $sent of ${fill.length} inputs');
    } finally {
      _localFree(inputs.cast());
    }
  }

  static void Function(Pointer<_Input>) _deviceKey(int vk, {required bool up}) => (input) {
    input.ref.type = _inputKeyboard;
    input.ref.input.keyboard
      ..vk = vk
      ..scan = _mapVirtualKey(vk, 0)
      ..flags = up ? _keyEventUp : 0;
  };

  /// Presses and releases the key [vk], with Shift, Ctrl or Alt held when asked (on [WindowsInputMode.device] only: a posted
  /// message carries no modifier state, which Windows reads from the keyboard itself).
  Future<void> stroke(int vk, {bool shift = false, bool ctrl = false, bool alt = false}) async {
    switch (mode) {
      case WindowsInputMode.device:
        final held = [if (shift) _vkShift, if (ctrl) _vkControl, if (alt) _vkMenu];
        _send([
          for (final each in held) _deviceKey(each, up: false),
          _deviceKey(vk, up: false),
          _deviceKey(vk, up: true),
          for (final each in held.reversed) _deviceKey(each, up: true),
        ]);
      case WindowsInputMode.posted:
        if (shift || ctrl || alt) throw const WindowsInputRefused('a posted key carries no Shift, Ctrl or Alt');
        // The release waits until the press has been taken from the queue, which is when the runner's TranslateMessage puts the
        // key's WM_CHAR after it, so the engine finds the character right behind the press as it does for a person's key; posted
        // together, the release would come between them and the key would arrive with no character.
        final scan = _mapVirtualKey(vk, 0) & 0xFF;
        _post(_wmKeyDown, vk, 1 | scan << 16);
        await Future<void>.delayed(const Duration(milliseconds: 30));
        _post(_wmKeyUp, vk, 1 | scan << 16 | 1 << 30 | 1 << 31);
    }
    await Future<void>.delayed(const Duration(milliseconds: 30));
  }

  /// Types [text] key by key, as the keyboard layout in use types each character.
  Future<void> type(String text) async {
    for (final unit in text.codeUnits) {
      final code = _vkKeyScan(unit);
      if (code == -1) throw WindowsInputRefused('no key of this keyboard types ${String.fromCharCode(unit)}');
      final modifiers = code >> 8 & 0xFF;
      await stroke(code & 0xFF, shift: modifiers & 1 != 0, ctrl: modifiers & 2 != 0, alt: modifiers & 4 != 0);
    }
  }

  /// Presses one of [windowsKeys].
  Future<void> key(String name) => stroke(windowsKeys[name]!);

  /// Clicks the left button at ([x], [y]), physical pixels from the top left of the view.
  Future<void> click(double x, double y) async {
    switch (mode) {
      case WindowsInputMode.posted:
        final at = (x.round() & 0xFFFF) | (y.round() & 0xFFFF) << 16;
        _post(_wmMouseMove, 0, at);
        await Future<void>.delayed(const Duration(milliseconds: 100));
        _post(_wmLeftDown, _mkLeft, at);
        await Future<void>.delayed(const Duration(milliseconds: 50));
        _post(_wmLeftUp, 0, at);
      case WindowsInputMode.device:
        final point = _localAlloc(0x40, sizeOf<_Point>()).cast<_Point>();
        final int screenX, screenY;
        try {
          point.ref
            ..x = x.round()
            ..y = y.round();
          _clientToScreen(view, point);
          (screenX, screenY) = (point.ref.x, point.ref.y);
        } finally {
          _localFree(point.cast());
        }
        // SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN: absolute input spans the whole desktop.
        final (left, top, width, height) = (_systemMetrics(76), _systemMetrics(77), _systemMetrics(78), _systemMetrics(79));
        final ax = ((screenX - left) * 65535 / (width - 1)).round();
        final ay = ((screenY - top) * 65535 / (height - 1)).round();
        void Function(Pointer<_Input>) mouse(int flags) => (input) {
          input.ref.type = _inputMouse;
          input.ref.input.mouse
            ..dx = ax
            ..dy = ay
            ..flags = _mouseAbsolute | _mouseVirtualDesk | _mouseMove | flags;
        };
        _send([mouse(0)]);
        await Future<void>.delayed(const Duration(milliseconds: 100));
        _send([mouse(_mouseLeftDown)]);
        await Future<void>.delayed(const Duration(milliseconds: 50));
        _send([mouse(_mouseLeftUp)]);
    }
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
}

/// [text] as a NUL-terminated UTF-16 string, freed with LocalFree.
Pointer<Uint16> _wide(String text) {
  final units = text.codeUnits;
  final out = _localAlloc(0x40, (units.length + 1) * 2).cast<Uint16>();
  for (final (index, unit) in units.indexed) {
    out[index] = unit;
  }
  return out;
}
