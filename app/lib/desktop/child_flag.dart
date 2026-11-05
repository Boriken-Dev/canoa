/// The first argument that makes the desktop app run as its host's own command when it starts itself. `main.dart` reads it
/// on every platform, a web build's included, so it is apart from what imports `dart:ffi`.
const String canoaChildFlag = '--canoa-child';
