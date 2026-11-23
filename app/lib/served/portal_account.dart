/// One of the portal's own pages as `GET /portal/account` describes it for the signed-in person.
final class PortalPage {
  const PortalPage({required this.path, required this.allowed, this.reason, this.state = const {}});

  /// Where the page is on the portal.
  final String path;

  /// Whether the portal lets this session do anything on the page now; false comes with a [reason].
  final bool allowed;

  /// Why the page offers nothing now, in the portal's own words.
  final String? reason;

  /// What stands in the page (`set_up`, `count`, `pinned`, `disabled`).
  final Map<String, Object?> state;

  /// Reads one entry of the answer; null where the entry is not an object with a path.
  static PortalPage? from(Object? entry) {
    if (entry is! Map || entry['path'] is! String) return null;
    return PortalPage(
      path: entry['path'] as String,
      allowed: entry['allowed'] == true,
      reason: entry['reason'] is String ? entry['reason'] as String : null,
      state: {for (final MapEntry(:key, :value) in entry.entries) if (key is String) key: value},
    );
  }
}

/// What the portal says the signed-in person may open now. A page the portal does not have is null.
final class PortalAccount {
  const PortalAccount({required this.user, this.code, this.passkeys, this.certificate, this.signOut});

  final String user;

  /// The one-time code's page.
  final PortalPage? code;
  final PortalPage? passkeys;
  final PortalPage? certificate;
  final PortalPage? signOut;

  /// Whether a one-time code is set up.
  bool get codeSetUp => code?.state['set_up'] == true;

  /// How many passkeys the account holds; null when the portal could not read them.
  int? get passkeyCount => passkeys?.state['count'] is int ? passkeys!.state['count'] as int : null;

  /// Whether a certificate is pinned to the account.
  bool get certificatePinned => certificate?.state['pinned'] == true;

  /// Reads the portal's answer; null where it is not the object it should be.
  static PortalAccount? from(Object? answer) {
    if (answer is! Map || answer['user'] is! String) return null;
    return PortalAccount(
      user: answer['user'] as String,
      code: PortalPage.from(answer['code']),
      passkeys: PortalPage.from(answer['passkeys']),
      certificate: PortalPage.from(answer['certificate']),
      signOut: PortalPage.from(answer['sign_out']),
    );
  }
}
