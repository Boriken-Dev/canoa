import 'package:coqui_web/coqui_web.dart';

Future<String> verifyBrowserPeer(WebChannel channel, String expectedPeerId) =>
    Future.error(UnsupportedError(
      'Browser WebCrypto is unavailable; use a native coqui-peer method',
    ));

Future<String> localBrowserPeerId() => Future.error(
  UnsupportedError('Native browser identity is unavailable'),
);
