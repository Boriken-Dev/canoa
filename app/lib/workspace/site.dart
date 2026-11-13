// The host name of a webpage as one safe folder name: lower case, IDN as punycode, no port, path or credentials, and nothing a
// file system reads as anything but a name. A name that cannot be made safe is refused, never repaired, so a crafted address
// cannot place a folder anywhere but under the scratch root.

/// The longest a whole host name may be, and its longest label (RFC 1035).
const _maxName = 253, _maxLabel = 63;

final _asciiLabel = RegExp(r'^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$');
final _reservedLabel = RegExp(r'^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])$');
final _ipv4 = RegExp(r'^(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)$');

/// The site [address] belongs to, as a folder name, or null when it is not an `http` or `https` address with a host name that
/// [siteName] accepts. [address] is a tab's address: `https://Example.COM:8443/a?b#c` gives `example.com`.
String? siteFqdn(String address) {
  final match = RegExp(r'^(https?)://([^/?#\\]*)', caseSensitive: false).firstMatch(address.trim());
  if (match == null) return null;
  var authority = match.group(2)!;
  // Credentials come before the last `@`; what follows is the host and an optional port.
  authority = authority.substring(authority.lastIndexOf('@') + 1);
  final port = RegExp(r':(\d{0,5})$').firstMatch(authority);
  if (port != null) authority = authority.substring(0, port.start);
  return siteName(authority);
}

/// [host] as a folder name, or null: lower case, a single trailing dot dropped, each non-ASCII label as `xn--` punycode.
/// Refused: an empty name, one over 253 characters, an empty label or one over 63, a label with anything but letters, digits and
/// inner hyphens (so no `..`, separator, `:`, percent sign, space or control character), an IPv6 address, and a first label
/// Windows reserves as a device name (`con`, `nul`, `com1`...), which it also reserves with an extension.
String? siteName(String host) {
  var name = host.toLowerCase();
  if (name.endsWith('.')) name = name.substring(0, name.length - 1);
  if (name.isEmpty) return null;
  final labels = <String>[];
  for (final label in name.split('.')) {
    if (label.isEmpty) return null;
    final ascii = label.runes.every((rune) => rune < 0x80) ? label : _punycodeLabel(label);
    if (ascii == null || ascii.length > _maxLabel || !_asciiLabel.hasMatch(ascii)) return null;
    labels.add(ascii);
  }
  final result = labels.join('.');
  if (result.length > _maxName) return null;
  if (_reservedLabel.hasMatch(labels.first)) return null;
  // Four numbers are an address, which is a fine name when it is a real one; anything else numeric all the way is not a host.
  if (labels.every((label) => RegExp(r'^\d+$').hasMatch(label)) && !_ipv4.hasMatch(result)) return null;
  return result;
}

/// The label as `xn--…`, or null when it holds a character an IDN label cannot (a control or format character, a space, a
/// separator or an ASCII symbol: only letters, digits and hyphens survive, as in a plain label).
String? _punycodeLabel(String label) {
  for (final rune in label.runes) {
    if (rune < 0x80) {
      if (!RegExp(r'[a-z0-9-]').hasMatch(String.fromCharCode(rune))) return null;
    } else if (rune < 0xa0 || (rune >= 0x2000 && rune <= 0x206f) || rune == 0xfeff || rune == 0x3000 || (rune >= 0xd800 && rune <= 0xdfff)) {
      return null;
    }
  }
  final encoded = punycodeEncode(label);
  return encoded == null ? null : 'xn--$encoded';
}

const _base = 36, _tMin = 1, _tMax = 26, _skew = 38, _damp = 700, _initialBias = 72, _initialN = 128;

int _adapt(int delta, int points, bool first) {
  var d = first ? delta ~/ _damp : delta ~/ 2;
  d += d ~/ points;
  var k = 0;
  while (d > ((_base - _tMin) * _tMax) ~/ 2) {
    d ~/= _base - _tMin;
    k += _base;
  }
  return k + (_base - _tMin + 1) * d ~/ (d + _skew);
}

String _digit(int value) => String.fromCharCode(value < 26 ? 0x61 + value : 0x30 + value - 26);

/// RFC 3492's Punycode of [input], without the `xn--` prefix; null when a value overflows.
String? punycodeEncode(String input) {
  final runes = input.runes.toList();
  final out = StringBuffer();
  for (final rune in runes) {
    if (rune < 0x80) out.writeCharCode(rune);
  }
  final basic = out.length;
  var handled = basic;
  if (basic > 0) out.write('-');
  var n = _initialN, delta = 0, bias = _initialBias;
  const limit = 0x7fffffff;
  while (handled < runes.length) {
    final m = runes.where((rune) => rune >= n).reduce((a, b) => a < b ? a : b);
    if ((m - n) * (handled + 1) > limit - delta) return null;
    delta += (m - n) * (handled + 1);
    n = m;
    for (final rune in runes) {
      if (rune < n && ++delta > limit) return null;
      if (rune == n) {
        var q = delta;
        for (var k = _base;; k += _base) {
          final t = k <= bias ? _tMin : (k >= bias + _tMax ? _tMax : k - bias);
          if (q < t) break;
          out.write(_digit(t + (q - t) % (_base - t)));
          q = (q - t) ~/ (_base - t);
        }
        out.write(_digit(q));
        bias = _adapt(delta, handled + 1, handled == basic);
        delta = 0;
        handled++;
      }
    }
    delta++;
    n++;
  }
  return out.toString();
}
