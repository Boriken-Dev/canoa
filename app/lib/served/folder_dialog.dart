import 'package:flutter/material.dart';

import '../selectable.dart';
import 'host_files.dart';

/// Lists one folder of the host (`fs/list`), or the places to start from for null.
typedef ListHostFolder = Future<HostFolderListing> Function(String? path);

/// Answers why [path] cannot be a workspace on the host, in its own words, or null when it can be.
typedef CheckHostFolder = Future<String?> Function(String path);

/// The folders above [path], from the root down, each with the path that names it: `/home/ana` is `/`, `/home`, `/home/ana`, and
/// `C:\work\app` is `C:\`, `C:\work`, `C:\work\app`. A path is taken as the host that listed it writes it.
List<({String label, String path})> breadcrumbs(String path) {
  final unc = path.startsWith(r'\\');
  final drive = RegExp(r'^[A-Za-z]:').hasMatch(path);
  final separator = unc || drive || (path.contains(r'\') && !path.startsWith('/')) ? r'\' : '/';
  final parts = path.split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty).toList();
  final out = <({String label, String path})>[];
  if (unc) {
    if (parts.isEmpty) return [(label: path, path: path)];
    var at = '$separator$separator${parts.first}';
    out.add((label: at, path: at));
    for (final part in parts.skip(1)) {
      at = '$at$separator$part';
      out.add((label: part, path: at));
    }
    return out;
  }
  var at = drive ? '${parts.isEmpty ? path.substring(0, 2) : parts.first}$separator' : separator;
  if (drive && parts.isNotEmpty) parts.removeAt(0);
  out.add((label: at, path: at));
  for (final part in parts) {
    at = at.endsWith(separator) ? '$at$part' : '$at$separator$part';
    out.add((label: part, path: at));
  }
  return out;
}

/// Asks which folder of the host's computer a session works in: the places the host lets this key browse, then the folders under
/// one. Answers the folder's path as the host names it, or null when the person closes the dialog.
///
/// [recent] are the folders sessions worked in on this host, newest first; [check] says why a folder cannot be chosen, asked of the
/// host before the dialog closes; [host] names the machine in the title.
Future<String?> pickHostFolder(
  BuildContext context,
  ListHostFolder list, {
  String? start,
  List<String> recent = const [],
  CheckHostFolder? check,
  String? host,
}) => showSelectableDialog<String>(
  context: context,
  builder: (_) => HostFolderDialog(list: list, start: start, recent: recent, check: check, host: host),
);

/// The folder dialog's body, apart from the route that shows it.
class HostFolderDialog extends StatefulWidget {
  const HostFolderDialog({super.key, required this.list, this.start, this.recent = const [], this.check, this.host});

  final ListHostFolder list;

  /// The folder to open first; the starting places when null or when the host will not list it.
  final String? start;
  final List<String> recent;
  final CheckHostFolder? check;
  final String? host;

  @override
  State<HostFolderDialog> createState() => _HostFolderDialogState();
}

class _HostFolderDialogState extends State<HostFolderDialog> {
  HostFolderListing? _listing;
  String? _problem;
  bool _loading = true;
  bool _editing = false;
  int _asked = 0;
  final _typed = TextEditingController();

  @override
  void initState() {
    super.initState();
    _go(widget.start, orPlaces: true);
  }

  @override
  void dispose() {
    _typed.dispose();
    super.dispose();
  }

  /// Lists [path]; with [orPlaces] a folder the host will not list falls back to the starting places.
  Future<void> _go(String? path, {bool orPlaces = false}) async {
    final asked = ++_asked;
    setState(() {
      _loading = true;
      _problem = null;
    });
    HostFolderListing? found;
    String? problem;
    try {
      found = await widget.list(path);
    } on Object catch (error) {
      problem = '$error';
      if (orPlaces && path != null) {
        try {
          found = await widget.list(null);
          problem = null;
        } on Object catch (again) {
          problem = '$again';
        }
      }
    }
    if (!mounted || asked != _asked) return;
    setState(() {
      _loading = false;
      _problem = problem;
      _editing = _editing && problem != null;
      if (found != null) _listing = found;
    });
  }

  /// Closes with [path] once the host agrees it can be a workspace.
  Future<void> _select(String path) async {
    final check = widget.check;
    if (check != null) {
      setState(() {
        _loading = true;
        _problem = null;
      });
      final problem = await check(path);
      if (!mounted) return;
      if (problem != null) {
        setState(() {
          _loading = false;
          _problem = problem;
        });
        return;
      }
    }
    if (mounted) Navigator.pop(context, path);
  }

  void _edit() {
    _typed.text = _listing?.path ?? '';
    _typed.selection = TextSelection(baseOffset: 0, extentOffset: _typed.text.length);
    setState(() => _editing = true);
  }

  @override
  Widget build(BuildContext context) {
    final listing = _listing;
    final path = listing?.path;
    final theme = Theme.of(context);
    return AlertDialog(
      key: const Key('host-folder-dialog'),
      title: Text(widget.host == null ? 'Choose a workspace' : 'Choose a workspace on ${widget.host}', overflow: TextOverflow.ellipsis),
      content: SizedBox(
        width: 460,
        height: 420,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(children: [
              IconButton(
                key: const Key('host-folder-places'),
                tooltip: 'Places',
                onPressed: _loading ? null : () => _go(null),
                icon: const Icon(Icons.home_outlined),
              ),
              IconButton(
                key: const Key('host-folder-up'),
                tooltip: 'Up one folder',
                // From a folder with nothing above it that this key may browse, back to the starting places.
                onPressed: _loading || listing == null || path == null ? null : () => _go(listing.parent),
                icon: const Icon(Icons.arrow_upward_rounded),
              ),
              IconButton(
                key: const Key('host-folder-refresh'),
                tooltip: 'Refresh',
                onPressed: _loading ? null : () => _go(path),
                icon: const Icon(Icons.refresh),
              ),
              Expanded(
                child: _editing
                    ? TextField(
                        key: const Key('host-folder-typed'),
                        controller: _typed,
                        autofocus: true,
                        decoration: const InputDecoration(isDense: true, hintText: 'A workspace on the host'),
                        onSubmitted: (typed) {
                          final wanted = typed.trim();
                          if (wanted.isNotEmpty) _go(wanted);
                        },
                      )
                    : Text(path ?? 'Where to start', key: const Key('host-folder-path'), overflow: TextOverflow.ellipsis),
              ),
              IconButton(
                key: const Key('host-folder-edit'),
                tooltip: _editing ? 'Stop typing a path' : 'Type a path',
                onPressed: _editing ? () => setState(() => _editing = false) : _edit,
                icon: Icon(_editing ? Icons.close : Icons.edit_outlined),
              ),
            ]),
            if (path != null && !_editing)
              SizedBox(
                height: 32,
                child: ListView(
                  key: const Key('host-folder-crumbs'),
                  scrollDirection: Axis.horizontal,
                  children: [
                    for (final crumb in breadcrumbs(path))
                      TextButton(
                        key: ValueKey('host-crumb ${crumb.path}'),
                        style: TextButton.styleFrom(minimumSize: const Size(32, 28), padding: const EdgeInsets.symmetric(horizontal: 6)),
                        onPressed: _loading || crumb.path == path ? null : () => _go(crumb.path),
                        child: Text(crumb.label),
                      ),
                  ],
                ),
              ),
            if (widget.recent.isNotEmpty)
              SizedBox(
                height: 40,
                child: ListView(
                  key: const Key('host-folder-recent'),
                  scrollDirection: Axis.horizontal,
                  children: [
                    for (final folder in widget.recent)
                      Padding(
                        padding: const EdgeInsets.only(right: 6, top: 4, bottom: 4),
                        child: ActionChip(
                          key: ValueKey('host-recent $folder'),
                          avatar: const Icon(Icons.history, size: 16),
                          label: Text(breadcrumbs(folder).last.label),
                          tooltip: folder,
                          onPressed: _loading ? null : () => _go(folder),
                        ),
                      ),
                  ],
                ),
              ),
            if (_problem != null)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 6),
                child: Text(_problem!, key: const Key('host-folder-problem'), style: TextStyle(color: theme.colorScheme.error)),
              ),
            const Divider(height: 1),
            Expanded(
              child: _loading && listing == null
                  ? const Center(child: CircularProgressIndicator())
                  : ListView(
                      children: [
                        for (final entry in listing?.entries ?? const <HostFolderEntry>[])
                          if (entry.isFolder)
                            ListTile(
                              key: ValueKey('host-folder ${entry.path}'),
                              dense: true,
                              leading: const Icon(Icons.folder_outlined),
                              title: Text(entry.name),
                              onTap: _loading ? null : () => _go(entry.path),
                            ),
                        if (listing != null && listing.entries.where((entry) => entry.isFolder).isEmpty)
                          const Padding(padding: EdgeInsets.all(12), child: Text('No folders here.')),
                        if (listing?.more ?? false)
                          const Padding(padding: EdgeInsets.all(12), child: Text('More folders are here than the host lists.')),
                      ],
                    ),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
        FilledButton(
          key: const Key('host-folder-choose'),
          onPressed: _loading || path == null ? null : () => _select(path),
          child: const Text('Select'),
        ),
      ],
    );
  }
}
