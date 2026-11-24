// The close button of a top-level window of this process, as Windows delivers it: WM_CLOSE posted to the window, which the engine
// hands to the window's own delegate. Windows only.
import 'dart:ffi';

final _user32 = DynamicLibrary.open('user32.dll');
final _kernel32 = DynamicLibrary.open('kernel32.dll');

final _enumWindows = _user32.lookupFunction<Int32 Function(Pointer<NativeFunction<Int32 Function(IntPtr, IntPtr)>>, IntPtr), int Function(Pointer<NativeFunction<Int32 Function(IntPtr, IntPtr)>>, int)>('EnumWindows');
final _enumChildWindows = _user32.lookupFunction<Int32 Function(IntPtr, Pointer<NativeFunction<Int32 Function(IntPtr, IntPtr)>>, IntPtr), int Function(int, Pointer<NativeFunction<Int32 Function(IntPtr, IntPtr)>>, int)>('EnumChildWindows');
final _getClassName = _user32.lookupFunction<Int32 Function(IntPtr, Pointer<Uint16>, Int32), int Function(int, Pointer<Uint16>, int)>('GetClassNameW');
final _getParent = _user32.lookupFunction<IntPtr Function(IntPtr), int Function(int)>('GetParent');
final _windowThreadProcessId = _user32.lookupFunction<Uint32 Function(IntPtr, Pointer<Uint32>), int Function(int, Pointer<Uint32>)>('GetWindowThreadProcessId');
final _getWindowText = _user32.lookupFunction<Int32 Function(IntPtr, Pointer<Uint16>, Int32), int Function(int, Pointer<Uint16>, int)>('GetWindowTextW');
final _postMessage = _user32.lookupFunction<Int32 Function(IntPtr, Uint32, IntPtr, IntPtr), int Function(int, int, int, int)>('PostMessageW');
final _currentProcessId = _kernel32.lookupFunction<Uint32 Function(), int Function()>('GetCurrentProcessId');
final _localAlloc = _kernel32.lookupFunction<Pointer<Void> Function(Uint32, IntPtr), Pointer<Void> Function(int, int)>('LocalAlloc');
final _localFree = _kernel32.lookupFunction<Pointer<Void> Function(Pointer<Void>), Pointer<Void> Function(Pointer<Void>)>('LocalFree');

const _wmClose = 0x0010;

/// The top-level windows of this process, by handle, with their titles.
Map<int, String> windowTitles() {
  final found = <int, String>{};
  final pid = _localAlloc(0x40, 4).cast<Uint32>();
  final buffer = _localAlloc(0x40, 1024).cast<Uint16>();
  final callback = NativeCallable<Int32 Function(IntPtr, IntPtr)>.isolateLocal((int window, int _) {
    _windowThreadProcessId(window, pid);
    if (pid.value == _currentProcessId()) {
      final length = _getWindowText(window, buffer, 512);
      found[window] = String.fromCharCodes([for (var i = 0; i < length; i++) buffer[i]]);
      final cls = _getClassName(window, buffer, 512);
      found[window] = '${found[window]} [${String.fromCharCodes([for (var i = 0; i < cls; i++) buffer[i]])}] parent=${_getParent(window)}';
    }
    return 1;
  }, exceptionalReturn: 0);
  try {
    _enumWindows(callback.nativeFunction, 0);
    for (final top in [...found.keys]) {
      _enumChildWindows(top, callback.nativeFunction, 0);
    }
  } finally {
    callback.close();
    _localFree(pid.cast());
    _localFree(buffer.cast());
  }
  return found;
}

/// Posts the close button's message to this process's top-level window titled [title]; false when there is none.
bool closeWindowTitled(String title) {
  for (final MapEntry(:key, :value) in windowTitles().entries) {
    if (value.startsWith(title)) return _postMessage(key, _wmClose, 0, 0) != 0;
  }
  return false;
}
