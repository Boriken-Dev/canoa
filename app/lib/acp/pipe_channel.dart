import 'pipe_channel_stub.dart' if (dart.library.io) 'pipe_channel_io.dart' as platform;
import 'package:coqui_web/coqui_web.dart';

/// Whether [url] names a pipe or a Unix socket, as `canoa serve` prints an entry that listens on one: `pipe:NAME` (a Windows
/// named pipe, or on Unix the socket `NAME.sock` in the runtime directory) or `unix:PATH`. A browser cannot dial either.
bool isPipeUrl(String url) => url.startsWith('pipe:') || url.startsWith('unix:');

/// The longest message either way: the host's own bound on an ACP line, and the overhead of the record around it.
const int pipeMaxMessage = 16 * 1024 * 1024 + 64 * 1024;

/// Opens the pipe or Unix socket [url] names, as a channel of whole messages (a little-endian `u32` length, then the body, the
/// stream format of WebTransport). It proves nobody: run the peer proof before any ACP message, as on any channel. The
/// operating system lets only this user in, which is no reason to skip the proof.
///
/// Throws [TransportUnavailable] when the pipe cannot be opened (nothing listens, the system refuses this user, or [timeout]
/// passes) or this platform has no pipes (the web build).
Future<ByteWebChannel> openPipeChannel(
  String url, {
  Duration timeout = const Duration(seconds: 10),
  int maxMessage = pipeMaxMessage,
}) {
  if (!isPipeUrl(url)) throw ArgumentError.value(url, 'url', 'not a pipe: or unix: address');
  return platform.openPipeChannel(url, timeout, maxMessage);
}
