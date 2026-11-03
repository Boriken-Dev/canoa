// A canoa-host whose `serve` and `pair` name `--method webcrypto`: the app's browser proof is WebCrypto only, and a new
// home would otherwise prove by Noise.
import 'package:canoa/desktop/host_tool.dart';

/// [args] with `--method webcrypto` after a `serve` or `pair` subcommand, and unchanged otherwise.
List<String> withWebCrypto(List<String> args) => switch (args) {
  ['serve' || 'pair', ...] => [args.first, '--method', 'webcrypto', ...args.skip(1)],
  _ => args,
};

/// [inner], with [withWebCrypto] applied to everything it runs.
final class WebCryptoHostTool implements HostTool {
  WebCryptoHostTool(this.inner);
  final HostTool inner;

  @override
  String get path => inner.path;

  @override
  Future<HostChild> start(List<String> args) => inner.start(withWebCrypto(args));

  @override
  Future<HostOutput> run(List<String> args) => inner.run(withWebCrypto(args));
}
