import 'package:flutter/material.dart';

import '../selectable.dart';
import 'host_files.dart';

/// Lists one folder of the host (`fs/list`), or the places to start from for null.
typedef ListHostFolder = Future<HostFolderListing> Function(String? path);

/// Asks which folder of the host's computer a session works in: the places the host lets this key browse, then the folders under
/// one. Answers the folder's path as the host names it, or null when the person closes the dialog.
Future<String?> pickHostFolder(BuildContext context, ListHostFolder list, {String? start}) => showSelectableDialog<String>(
  context: context,
  builder: (_) => HostFolderDialog(list: list, start: start),
);

/// The folder dialog's body, apart from the route that shows it.
class HostFolderDialog extends StatefulWidget {
  const HostFolderDialog({super.key, required this.list, this.start});

  final ListHostFolder list;

  /// The folder to open first; the starting places when null or when the host will not list it.
  final String? start;

  @override
  State<HostFolderDialog> createState() => _HostFolderDialogState();
}

class _HostFolderDialogState extends State<HostFolderDialog> {
  HostFolderListing? _listing;
  String? _problem;
  bool _loading = true;
  int _asked = 0;

  @override
  void initState() {
    super.initState();
    _go(widget.start, orPlaces: true);
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
      if (found != null) _listing = found;
    });
  }

  @override
  Widget build(BuildContext context) {
    final listing = _listing;
    final path = listing?.path;
    return AlertDialog(
      key: const Key('host-folder-dialog'),
      title: const Text('Choose a workspace'),
      content: SizedBox(
        width: 420,
        height: 380,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(children: [
              IconButton(
                key: const Key('host-folder-up'),
                tooltip: 'Up one folder',
                // From a folder with nothing above it that this key may browse, back to the starting places.
                onPressed: _loading || listing == null || path == null ? null : () => _go(listing.parent),
                icon: const Icon(Icons.arrow_upward_rounded),
              ),
              Expanded(
                child: Text(path ?? 'Where to start', key: const Key('host-folder-path'), overflow: TextOverflow.ellipsis),
              ),
            ]),
            if (_problem != null)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 6),
                child: Text(_problem!, key: const Key('host-folder-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
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
          onPressed: _loading || path == null ? null : () => Navigator.pop(context, path),
          child: const Text('Choose this workspace'),
        ),
      ],
    );
  }
}
