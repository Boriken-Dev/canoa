// The Noise method's other side, from the module built with tool/build_noise.py --test-peer (it is not in the app's build).
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:canoa/auth/web/noise_method.dart';

extension type NoiseResponder._(JSObject _) implements JSObject {
  external JSArray<JSUint8Array> step(JSUint8Array received);
  external String? get peerId;
  external ServerSession? takeSession();
}

/// A session as the module exports it, which is the responder's.
extension type ServerSession._(JSObject _) implements JSObject {
  external JSUint8Array seal(JSUint8Array message);
  external JSUint8Array? open(JSUint8Array record);
  external JSUint8Array close();
}

/// A responder key of its own, with its id.
final class ResponderSide {
  ResponderSide._(this._module, this._key, this.peerId);

  final JSObject _module;
  final JSObject _key;
  final String peerId;

  static Future<ResponderSide> create() async {
    final module = await loadNoise();
    final key = (module['NoiseKey']! as JSObject).callMethod<JSObject>('generate'.toJS);
    return ResponderSide._(module, key, (key['peerId']! as JSString).toDart);
  }

  /// A responder that admits the dialer [allowed] alone.
  NoiseResponder admit(String allowed) =>
      (_module['NoiseResponder']! as JSObject).callMethod<NoiseResponder>('new'.toJS, _key, allowed.toJS);
}
