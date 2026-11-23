import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../served/portal_account.dart';
import '../served/portal_session.dart';
import 'settings_row.dart';

/// Settings' Sign-in page, for a page the portal served: where the person sets up their one-time code, adds a passkey, enrols a
/// certificate and signs out. Each is a page of the portal's own, opened in this tab. What each says is what the portal says the
/// person may do now ([PortalSession.account]), so a page it will not let them use here is shown with its reason and no button.
class SignInSection extends StatefulWidget {
  const SignInSection({super.key, required this.portal});

  final PortalSession portal;

  @override
  State<SignInSection> createState() => _SignInSectionState();
}

class _SignInSectionState extends State<SignInSection> {
  late Future<PortalAccount?> _account = widget.portal.account();

  @override
  void didUpdateWidget(SignInSection oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.portal != widget.portal) _account = widget.portal.account();
  }

  @override
  Widget build(BuildContext context) => FutureBuilder<PortalAccount?>(
    future: _account,
    builder: (context, snapshot) {
      if (snapshot.connectionState != ConnectionState.done) {
        return const Padding(padding: EdgeInsets.all(16), child: Center(child: CircularProgressIndicator()));
      }
      final account = snapshot.data;
      if (account == null) {
        return Padding(
          padding: const EdgeInsets.all(16),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text(context.t('signin.unreadable'), key: const Key('signin-unreadable')),
            const SizedBox(height: 8),
            OutlinedButton(
              key: const Key('signin-retry'),
              onPressed: () => setState(() {
                _account = widget.portal.account();
              }),
              child: Text(context.t('signin.retry')),
            ),
          ]),
        );
      }
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
          child: Text(context.t('signin.signed_in_as', {'account': account.user}), key: const Key('signin-user')),
        ),
        if (account.code case final page?)
          _row(
            context,
            id: 'code',
            page: page,
            title: context.t('signin.code'),
            status: context.t(account.codeSetUp ? 'signin.code.set' : 'signin.code.unset'),
            action: context.t(account.codeSetUp ? 'signin.code.again' : 'signin.code.setup'),
          ),
        if (account.passkeys case final page?)
          _row(
            context,
            id: 'passkeys',
            page: page,
            title: context.t('signin.passkeys'),
            status: switch (account.passkeyCount) {
              null => context.t('signin.passkeys.unknown'),
              0 => context.t('signin.passkeys.none'),
              1 => context.t('signin.passkeys.one'),
              final count => context.t('signin.passkeys.many', {'count': count}),
            },
            action: context.t((account.passkeyCount ?? 0) > 0 ? 'signin.passkeys.manage' : 'signin.passkeys.add'),
          ),
        if (account.certificate case final page?)
          _row(
            context,
            id: 'certificate',
            page: page,
            title: context.t('signin.certificate'),
            status: context.t(account.certificatePinned ? 'signin.certificate.pinned' : 'signin.certificate.none'),
            action: context.t(account.certificatePinned ? 'signin.certificate.manage' : 'signin.certificate.enrol'),
          ),
        SettingsRow(
          key: const Key('signin-signout'),
          title: context.t('signin.signout'),
          description: context.t('signin.signout.help'),
          control: OutlinedButton(
            key: const Key('signin-signout-button'),
            onPressed: widget.portal.signOut,
            child: Text(context.t('shell.sign_out')),
          ),
        ),
      ]);
    },
  );

  /// A page's row: its status and a button that opens it, or, when the portal says it offers nothing now, the portal's reason.
  Widget _row(
    BuildContext context, {
    required String id,
    required PortalPage page,
    required String title,
    required String status,
    required String action,
  }) {
    final reason = page.allowed ? null : (page.reason ?? context.t('signin.not_now'));
    return SettingsRow(
      key: Key('signin-$id'),
      title: title,
      description: reason == null ? status : '$status. $reason',
      control: reason != null
          ? null
          : FilledButton.tonal(key: Key('signin-$id-open'), onPressed: () => widget.portal.open(page.path), child: Text(action)),
    );
  }
}
