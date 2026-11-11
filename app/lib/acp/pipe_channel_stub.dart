import 'package:coqui_web/coqui_web.dart';

/// A browser has no pipes.
Future<ByteWebChannel> openPipeChannel(String url, Duration timeout, int maxMessage) =>
    Future.error(const TransportUnavailable('A web page cannot open a pipe'));
