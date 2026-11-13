// The host name a webpage's folder is named for: lower case, punycode, nothing but a name, and nothing that can leave the scratch
// root. Each way a crafted name could do that is refused, and none is repaired.
import 'package:canoa/workspace/scratch.dart';
import 'package:canoa/workspace/site.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('the site of an address', () {
    test('is its host name in lower case, without scheme, port, path, query, fragment or credentials', () {
      expect(siteFqdn('https://example.com/a'), 'example.com');
      expect(siteFqdn('HTTP://Example.COM:8443/a/b?c=d#e'), 'example.com');
      expect(siteFqdn('https://user:secret@Docs.Example.com/x'), 'docs.example.com');
      expect(siteFqdn('https://example.com?x'), 'example.com');
      expect(siteFqdn('https://example.com#x'), 'example.com');
      expect(siteFqdn('https://example.com.'), 'example.com', reason: 'a trailing dot names the same host');
      expect(siteFqdn('  https://example.com/  '), 'example.com');
      expect(siteFqdn('http://localhost:3000/'), 'localhost');
      expect(siteFqdn('http://127.0.0.1:8080/'), '127.0.0.1');
    });

    test('takes the host after the last @, so credentials cannot name another site', () {
      expect(siteFqdn('https://evil.test@example.com/'), 'example.com');
      expect(siteFqdn('https://example.com@evil.test/'), 'evil.test');
      expect(siteFqdn(r'https://example.com\@evil.test/'), 'example.com', reason: 'a backslash ends the authority, as in a browser');
    });

    test('writes an international name as punycode', () {
      expect(siteFqdn('https://bücher.example/'), 'xn--bcher-kva.example');
      expect(siteFqdn('https://MÜNCHEN.de/'), 'xn--mnchen-3ya.de');
      expect(siteFqdn('https://日本語.jp/'), 'xn--wgv71a119e.jp');
      expect(siteFqdn('https://xn--bcher-kva.example/'), 'xn--bcher-kva.example', reason: 'already punycode, as a browser reports it');
      expect(punycodeEncode('ü'), 'tda');
      expect(punycodeEncode('abc'), 'abc-');
    });

    test('is none for an address that is not http or https', () {
      for (final address in [
        'file:///C:/Work/a.html', 'chrome://extensions/', 'chrome-extension://abc/page.html', 'about:blank', 'data:text/html,hi',
        'javascript:alert(1)', 'ftp://example.com/', 'example.com', '//example.com/', '', 'https:///a', 'https://',
      ]) {
        expect(siteFqdn(address), isNull, reason: address);
      }
    });
  });

  group('a host name is refused, never repaired, when it holds', () {
    void refused(String why, List<String> names) {
      test(why, () {
        for (final name in names) {
          expect(siteName(name), isNull, reason: 'name "$name"');
          // An address with one of these ends its host there, so what follows is not part of the name.
          if (!RegExp(r'[/\\?#:@]').hasMatch(name)) expect(siteFqdn('https://$name/'), isNull, reason: 'address of "$name"');
        }
      });
    }

    refused('a parent step', ['..', '.', 'a..b', 'example.com..', '..example.com', '.example.com', 'a.b/..', '../..', 'a.%2e%2e', '...']);
    refused('a separator', ['a/b', 'a\\b', '/etc', '\\\\server\\share', 'a/../b', 'example.com/..']);
    refused('a drive letter or a colon', ['c:', 'C:\\', 'c:\\windows', 'c:/', 'a:b', 'example.com:80:81']);
    refused('a name Windows reserves for a device', [
      'con', 'CON', 'prn', 'aux', 'nul', 'com1', 'COM9', 'lpt1', 'LPT9', 'nul.example.com', 'con.test', 'aux.a.b.c', 'com5.example',
    ]);
    refused('anything but letters, digits and inner hyphens', [
      'a b.com', ' ', 'a%2fb.com', 'a%2e%2e.com', 'exa_mple.com', '-a.com', 'a-.com', 'a.-b.com', 'a*.com', 'a?.com', 'a<b>.com',
      'a|b.com', 'a"b.com', 'a\u0000b.com', 'a\nb.com', 'a\tb.com', 'a\u007fb.com', 'a\u200bb.com', 'a\u202eb.com', 'a\u0085b.com', 'a!b.com',
    ]);
    refused('an IPv6 address, a bracket, or only numbers that are not an address', ['[::1]', '::1', '[fe80::1]', '1', '1.2.3', '999.1.1.1', '1.2.3.4.5', '01.2.3.4', '300']);
    refused('no name at all', ['', '.', ' ']);

    test('more than 253 characters, or a label of more than 63', () {
      final label63 = 'a' * 63;
      expect(siteName(label63), label63);
      expect(siteName('${'a' * 64}.com'), isNull);
      expect(siteName('$label63.$label63.$label63.${'b' * 61}'), hasLength(253));
      expect(siteName('$label63.$label63.$label63.${'b' * 62}'), isNull);
    });

    test('a label whose punycode is too long, or an international name that is not a name', () {
      expect(siteName('${'é' * 70}.com'), isNull);
      expect(siteName('é ü.com'), isNull);
      expect(siteName('é/ü.com'), isNull);
      expect(siteName('é\u200bü.com'), isNull);
      expect(siteName('é\u202e.com'), isNull);
    });
  });

  group('a folder for a site', () {
    String folder(String fqdn) => scratchFolder(
          r'${documents}/Canoa/scratch/${fqdn}/${session}', '3f2a9c1e-7b4d-4e58-9a16-0d5c2b8e6f40',
          documents: '/home/ana/Documents', home: '/home/ana', host: 'desk', separator: '/', fqdn: fqdn,
        )!;

    test('is one folder for the site and one for the session, under the scratch root', () {
      expect(folder('example.com'), '/home/ana/Documents/Canoa/scratch/example.com/3f2a9c1e-7b4d-4e58-9a16-0d5c2b8e6f40');
      expect(folder('xn--bcher-kva.example'), '/home/ana/Documents/Canoa/scratch/xn--bcher-kva.example/3f2a9c1e-7b4d-4e58-9a16-0d5c2b8e6f40');
    });

    test('is never made for a name that siteName would not write, so no name can leave the scratch root', () {
      for (final name in [
        '..', '../..', 'a/../..', 'a\\..\\..', '/etc', 'c:', 'C:\\Windows', 'con', 'nul.example.com', 'Example.com', 'example.com.', 'a b',
        'a\u0000b', '', '%2e%2e', '[::1]', '..\\..\\x',
      ]) {
        expect(() => folder(name), throwsArgumentError, reason: 'name "$name"');
      }
    });

    test('every address that names a site yields a folder under the root and nowhere else', () {
      for (final address in ['https://example.com/', 'https://a.b.c.d.example/x', 'http://1.2.3.4/', 'https://xn--mnchen-3ya.de/']) {
        final path = folder(siteFqdn(address)!);
        expect(path, startsWith('/home/ana/Documents/Canoa/scratch/'));
        expect(path.substring('/home/ana/Documents/Canoa/scratch/'.length).split('/'), hasLength(2));
        expect(path, isNot(contains('..')));
      }
    });

    test('is written with the separator of the machine its folder comes from', () {
      expect(separatorOf('/home/ana/Documents'), '/');
      expect(separatorOf(r'C:\Work\Documents'), r'\');
      expect(separatorOf(r'\\server\share\Documents'), r'\');
      expect(separatorOf('D:/Docs'), r'\');
    });
  });
}
