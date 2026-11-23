import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../store/links.dart' show openLink;
import '../theme/look.dart';
import '../worktree/worktree.dart' show WorktreeFailure;
import 'git_changes.dart';

/// A commit the host made.
final class GitCommitted {
  const GitCommitted({required this.commit, required this.summary, required this.branch});
  final String commit, summary;
  final String? branch;
}

/// What the host did to open a pull request.
final class PullRequestOpened {
  const PullRequestOpened({required this.viaGh, required this.url, required this.branch});

  /// Whether `gh` made it: [url] is the pull request. Otherwise [url] is the forge's page that starts one, for the person's browser.
  final bool viaGh;
  final String url, branch;
}

/// Finishes the work in a session's folder with the person's own git and `gh`, run by the host: every refusal is a
/// [WorktreeFailure] carrying git's or `gh`'s own words.
abstract interface class GitPublisher {
  /// Commits [paths] (relative to [folder]; everything under it when null) with [message].
  Future<GitCommitted> commit(String folder, String message, {List<String>? paths});

  /// Pushes the branch checked out in [folder]; [setUpstream] makes it track [remote] (`origin` when null).
  Future<void> push(String folder, {String? remote, bool setUpstream = false});

  /// Opens a pull request for the branch checked out in [folder].
  Future<PullRequestOpened> pullRequest(String folder, {required String title, String body = '', String? base, bool draft = false});
}

/// Commit, Push and Pull request for the repository of a session's folder, with the files to commit chosen one by one. Every
/// answer, git's own words when one refuses, stays on screen.
class GitPublishBar extends StatefulWidget {
  const GitPublishBar({
    super.key,
    required this.publisher,
    required this.folder,
    required this.files,
    required this.more,
    required this.canCommit,
    required this.branch,
    required this.onChanged,
  });

  final GitPublisher publisher;
  final String folder;

  /// The files a commit would take: what the disk holds against HEAD.
  final List<GitFile> files;

  /// Whether more files differ than [files] lists.
  final bool more;

  /// Whether Commit is offered: the view shows the disk against HEAD.
  final bool canCommit;

  /// The branch checked out; null when HEAD is detached.
  final String? branch;

  /// Called after the repository changed, so what is shown is read again.
  final Future<void> Function() onChanged;

  @override
  State<GitPublishBar> createState() => _GitPublishBarState();
}

class _GitPublishBarState extends State<GitPublishBar> {
  final _message = TextEditingController();
  final _remote = TextEditingController();
  final _unchosen = <String>{};
  bool _setUpstream = false;
  bool _filesOpen = false;
  String? _busy;
  String? _result;
  bool _failed = false;
  String _lastSummary = '';

  @override
  void dispose() {
    _message.dispose();
    _remote.dispose();
    super.dispose();
  }

  List<GitFile> get _chosen => [for (final file in widget.files) if (!_unchosen.contains(file.path)) file];

  Future<void> _run(String name, Future<String> Function() work) async {
    setState(() {
      _busy = name;
      _result = null;
    });
    String? said;
    var failed = false;
    try {
      said = await work();
    } on WorktreeFailure catch (error) {
      said = error.message;
      failed = true;
    } on Object catch (error) {
      said = error.toString();
      failed = true;
    }
    if (!failed) {
      try {
        await widget.onChanged();
      } on Object {
        // The next read says what it can; the answer above stands.
      }
    }
    if (!mounted) return;
    setState(() {
      _busy = null;
      _result = said;
      _failed = failed;
    });
  }

  Future<void> _commit() {
    final t = context.t;
    return _run('commit', () async {
    final chosen = _chosen;
    final all = chosen.length == widget.files.length && !widget.more;
    final made = await widget.publisher.commit(widget.folder, _message.text, paths: all ? null : [for (final file in chosen) file.path]);
    _message.clear();
    _unchosen.clear();
    _lastSummary = made.summary;
    return t('publish.committed', {'commit': made.commit.length > 7 ? made.commit.substring(0, 7) : made.commit, 'summary': made.summary});
  });
  }

  Future<void> _push() {
    final t = context.t;
    return _run('push', () async {
    final remote = _remote.text.trim();
    await widget.publisher.push(widget.folder, remote: remote.isEmpty ? null : remote, setUpstream: _setUpstream);
    final branch = widget.branch ?? t('publish.the_branch');
    return _setUpstream ? t('publish.pushed_upstream', {'branch': branch, 'remote': remote.isEmpty ? 'origin' : remote}) : t('publish.pushed', {'branch': branch});
  });
  }

  Future<void> _pullRequest() async {
    final t = context.t;
    final asked = await showDialog<_PullRequestForm>(context: context, builder: (_) => _PullRequestDialog(title: _lastSummary));
    if (asked == null || !mounted) return;
    await _run('pull request', () async {
      final made = await widget.publisher.pullRequest(widget.folder, title: asked.title, body: asked.body, base: asked.base, draft: asked.draft);
      if (made.viaGh) return t('publish.pr_opened', {'url': made.url});
      final uri = Uri.tryParse(made.url);
      final opened = uri != null && await openLink(uri);
      return opened ? t('publish.pr_page_opened', {'branch': made.branch, 'url': made.url}) : t('publish.pr_page', {'url': made.url});
    });
  }

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final muted = TextStyle(color: look.muted, fontSize: 12);
    final t = context.t;
    final busy = _busy != null;
    final chosen = _chosen.length;
    return Material(
      color: look.rail,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 340),
        child: SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 12),
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
            const Divider(height: 1),
            const SizedBox(height: 8),
            if (widget.canCommit && widget.files.isNotEmpty) ...[
              InkWell(
                key: const ValueKey('publish files'),
                onTap: () => setState(() => _filesOpen = !_filesOpen),
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  child: Row(children: [
                    Expanded(child: Text(t(widget.files.length == 1 ? 'publish.chosen_one' : 'publish.chosen', {'chosen': chosen, 'count': widget.files.length}), key: const ValueKey('publish chosen'), style: const TextStyle(fontSize: 13))),
                    Icon(_filesOpen ? Icons.expand_less : Icons.expand_more, size: 18),
                  ]),
                ),
              ),
              if (_filesOpen)
                for (final file in widget.files)
                  CheckboxListTile(
                    key: ValueKey('publish file ${file.path}'),
                    dense: true,
                    visualDensity: VisualDensity.compact,
                    contentPadding: EdgeInsets.zero,
                    controlAffinity: ListTileControlAffinity.leading,
                    value: !_unchosen.contains(file.path),
                    title: Text(file.path, style: const TextStyle(fontSize: 12), overflow: TextOverflow.ellipsis),
                    onChanged: busy ? null : (on) => setState(() => on == true ? _unchosen.remove(file.path) : _unchosen.add(file.path)),
                  ),
              TextField(
                key: const ValueKey('publish message'),
                controller: _message,
                minLines: 1,
                maxLines: 4,
                enabled: !busy,
                onChanged: (_) => setState(() {}),
                decoration: InputDecoration(hintText: t('publish.message'), isDense: true),
              ),
              const SizedBox(height: 6),
              Align(
                alignment: Alignment.centerLeft,
                child: FilledButton(
                  key: const ValueKey('publish commit'),
                  onPressed: busy || chosen == 0 || _message.text.trim().isEmpty ? null : _commit,
                  child: Text(t(_busy == 'commit' ? 'publish.committing' : 'publish.commit')),
                ),
              ),
              const SizedBox(height: 8),
            ],
            Wrap(spacing: 8, runSpacing: 4, crossAxisAlignment: WrapCrossAlignment.center, children: [
              OutlinedButton(
                key: const ValueKey('publish push'),
                onPressed: busy ? null : _push,
                child: Text(t(_busy == 'push' ? 'publish.pushing' : 'publish.push')),
              ),
              OutlinedButton(
                key: const ValueKey('publish pull request'),
                onPressed: busy ? null : _pullRequest,
                child: Text(t(_busy == 'pull request' ? 'publish.opening' : 'publish.pull_request')),
              ),
              Row(mainAxisSize: MainAxisSize.min, children: [
                Checkbox(key: const ValueKey('publish set upstream'), value: _setUpstream, visualDensity: VisualDensity.compact, onChanged: busy ? null : (on) => setState(() => _setUpstream = on ?? false)),
                Text(t('publish.set_upstream'), style: muted),
              ]),
              SizedBox(
                width: 110,
                child: TextField(
                  key: const ValueKey('publish remote'),
                  controller: _remote,
                  enabled: !busy,
                  style: const TextStyle(fontSize: 12),
                  decoration: InputDecoration(hintText: t('publish.remote_hint'), isDense: true),
                ),
              ),
            ]),
            if (_result != null) ...[
              const SizedBox(height: 8),
              SelectableText(_result!, key: const ValueKey('publish result'), style: TextStyle(fontSize: 12, color: _failed ? look.danger : null)),
            ],
          ]),
        ),
      ),
    );
  }
}

/// What the Pull request dialog asks.
final class _PullRequestForm {
  const _PullRequestForm({required this.title, required this.body, required this.base, required this.draft});
  final String title, body;
  final String? base;
  final bool draft;
}

class _PullRequestDialog extends StatefulWidget {
  const _PullRequestDialog({required this.title});
  final String title;

  @override
  State<_PullRequestDialog> createState() => _PullRequestDialogState();
}

class _PullRequestDialogState extends State<_PullRequestDialog> {
  late final _title = TextEditingController(text: widget.title);
  final _body = TextEditingController();
  final _base = TextEditingController();
  bool _draft = false;

  @override
  void dispose() {
    _title.dispose();
    _body.dispose();
    _base.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.t;
    return AlertDialog(
    title: Text(t('publish.pr_title')),
    content: SizedBox(
      width: 420,
      child: SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          TextField(key: const ValueKey('pr title'), controller: _title, onChanged: (_) => setState(() {}), decoration: InputDecoration(labelText: t('publish.pr_field_title'))),
          const SizedBox(height: 8),
          TextField(key: const ValueKey('pr body'), controller: _body, minLines: 3, maxLines: 8, decoration: InputDecoration(labelText: t('publish.pr_field_body'))),
          const SizedBox(height: 8),
          TextField(key: const ValueKey('pr base'), controller: _base, decoration: InputDecoration(labelText: t('publish.pr_field_base'), hintText: t('publish.pr_base_hint'))),
          CheckboxListTile(
            key: const ValueKey('pr draft'),
            contentPadding: EdgeInsets.zero,
            controlAffinity: ListTileControlAffinity.leading,
            value: _draft,
            title: Text(t('publish.pr_draft')),
            onChanged: (on) => setState(() => _draft = on ?? false),
          ),
        ]),
      ),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: Text(t('common.cancel'))),
      FilledButton(
        key: const ValueKey('pr create'),
        onPressed: _title.text.trim().isEmpty ? null : () => Navigator.pop(context, _PullRequestForm(title: _title.text.trim(), body: _body.text, base: _base.text.trim().isEmpty ? null : _base.text.trim(), draft: _draft)),
        child: Text(t('publish.pr_open')),
      ),
    ],
  );
  }
}
