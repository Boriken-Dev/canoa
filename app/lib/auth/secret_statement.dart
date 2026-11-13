import 'dart:convert';
import 'dart:typed_data';

/// The two labels a secret binding's statement begins with, each with its 4-byte length.
final List<Uint8List> _statementLabels = [
  for (final label in const ['dev.boriken.coqui.acp/secret/v1 binding', 'dev.boriken.coqui.acp/secret/v1 values'])
    Uint8List.fromList([0, 0, 0, label.length, ...ascii.encode(label)]),
];

/// Whether [message] begins as a secret binding's statement does. The identity key that proves this app to a host also signs these, so
/// nothing else may be signed with it through a signer that leaves `auth/`: a handshake's bytes never begin this way.
bool isSecretStatement(Uint8List message) => _statementLabels.any((prefix) {
  if (message.length < prefix.length) return false;
  for (var i = 0; i < prefix.length; i++) {
    if (message[i] != prefix[i]) return false;
  }
  return true;
});
