import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ComposerPill;
import 'package:material_ui/material_ui.dart';

import '../accounts/accounts_controller.dart';
import '../accounts/add_account_dialog.dart';
import '../providers/provider_catalog.dart';
import '../selectable.dart';
import '../session_store.dart';
import '../sources/source_choices.dart';
import '../sources/sources.dart';
import '../store/programs_view.dart';
import 'composer.dart' show ComposerKit;

/// Why a source's agent cannot start on this host, in the three ways the host tells apart.
enum TroubleKind {
  /// Its program is not on the host.
  notInstalled('Install'),

  /// Its program is there and nobody is signed in to it.
  signIn('Sign in'),

  /// Its program is there and could not be asked.
  failed('Try again');

  const TroubleKind(this.action);

  /// What the button that puts it right says.
  final String action;
}

/// A source that cannot start, and what to say and offer about it.
final class SourceTrouble {
  const SourceTrouble(this.kind, this.source, this.message, {this.detail});

  final TroubleKind kind;
  final AgentSource source;

  /// What the pill reads.
  final String message;

  /// The host's own words, for the tooltip, when it gave any.
  final String? detail;

  /// Whether starting a session now would only fail, so that sending offers the action instead.
  bool get blocksStart => kind != TroubleKind.failed;

  /// The program's name as a person knows it.
  static String programName(AgentSource source) =>
      (source.harness == null ? null : providerCatalog.byId(source.harness)?.program) ?? source.displayName;
}

/// What is wrong with [source], from what the host listed for it; null when nothing is, or the host has not said. A source's own
/// login only counts when no account is offered to it: an account carries its own.
SourceTrouble? sourceTrouble(AgentSource? source, SourceChoices? choices, {AccountsController? accounts}) {
  if (source == null || choices == null) return null;
  switch (choices.unknownKind) {
    case UnknownKind.notInstalled:
      return SourceTrouble(
        TroubleKind.notInstalled, source, "${SourceTrouble.programName(source)} isn't installed on this host",
        detail: choices.unknownBecause,
      );
    case UnknownKind.failed:
      return SourceTrouble(TroubleKind.failed, source, "Couldn't start ${source.displayName}", detail: choices.unknownBecause);
    case null:
      break;
  }
  final entry = source.hostEntry;
  if (choices.signedOut && (entry == null || (accounts?.offeredBy(entry).isEmpty ?? true))) {
    return SourceTrouble(TroubleKind.signIn, source, '${source.displayName} needs a sign-in');
  }
  return null;
}

/// Whether [kit] can put [trouble] right from here: the Store when the host's grants allow installing, Accounts when the app has
/// them. A failure can always be asked again.
bool canAct(SourceTrouble trouble, ComposerKit kit, {ChatSession? session}) => switch (trouble.kind) {
  TroubleKind.notInstalled => kit.programs != null,
  TroubleKind.signIn => kit.accountsFor(session) != null,
  TroubleKind.failed => true,
};

/// Runs [trouble]'s action: the Store's programs to install, Accounts' sign-in as it runs there, or asking the host again.
/// Whatever it did, the host is asked once more what the source offers.
Future<void> actOnTrouble(BuildContext context, SessionStore store, ComposerKit kit, SourceTrouble trouble, {ChatSession? session}) async {
  switch (trouble.kind) {
    case TroubleKind.notInstalled:
      final programs = kit.programs;
      if (programs == null) return;
      await Navigator.of(context).push<void>(selectablePage(
        builder: (_) => Scaffold(
          appBar: AppBar(title: const Text('Agent programs')),
          body: ListView(padding: const EdgeInsets.all(16), children: [ProgramsSection(programs: programs)]),
        ),
      ));
    case TroubleKind.signIn:
      final accounts = kit.accountsFor(session);
      if (accounts == null || !context.mounted) return;
      await showDialog<void>(
        context: context,
        builder: (_) => AddAccountDialog(accounts: accounts, provider: accounts.providers.where((each) => each.agent == trouble.source.harness).firstOrNull?.id),
      );
      unawaited(accounts.refresh());
    case TroubleKind.failed:
      break;
  }
  await store.askChoicesAgain(trouble.source);
}

/// A draft's source that cannot start, said in the composer's row beside its model: the reason, and the one thing to do about it.
class SourceTroubleChip extends StatelessWidget {
  const SourceTroubleChip({super.key, required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    if (!session.isDraft) return const SizedBox.shrink();
    final source = session.source;
    final trouble = sourceTrouble(source, source == null ? null : store.choicesOf(source), accounts: kit.accountsFor(session));
    if (trouble == null) return const SizedBox.shrink();
    final act = canAct(trouble, kit, session: session) ? () => unawaited(actOnTrouble(context, store, kit, trouble, session: session)) : null;
    return Row(mainAxisSize: MainAxisSize.min, children: [
      Tooltip(
        message: trouble.detail ?? trouble.message,
        child: InkWell(
          key: const Key('source-trouble'),
          borderRadius: BorderRadius.circular(8),
          onTap: act,
          child: ComposerPill(label: trouble.message, icon: Icons.error_outline, danger: true, maxWidth: 280),
        ),
      ),
      if (act != null)
        TextButton(key: const Key('source-trouble-action'), onPressed: act, child: Text(trouble.kind.action)),
    ]);
  }
}
