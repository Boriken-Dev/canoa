import 'package:material_ui/material_ui.dart';

import '../config/connection.dart';
import '../selectable.dart';
import '../workspace/remote_host.dart';

/// What the host picker answers: a host chosen, or the way out to Add connection or Hosts.
sealed class HostPick {
  const HostPick();
}

/// A host chosen: its saved connection's name, or null for this app's own.
final class HostChosen extends HostPick {
  const HostChosen(this.name);
  final String? name;
}

final class HostAdd extends HostPick {
  const HostAdd();
}

final class HostManage extends HostPick {
  const HostManage();
}

/// Asks where a new session starts: the hosts in [hosts], searchable, with Add and Manage that leave to their own pages.
Future<HostPick?> pickHost(BuildContext context, List<HostChoice> hosts, {required String? chosen, bool canAdd = true, bool canManage = true}) =>
    showSelectableDialog<HostPick>(
      context: context,
      builder: (_) => HostPicker(hosts: hosts, chosen: chosen, canAdd: canAdd, canManage: canManage),
    );

class HostPicker extends StatefulWidget {
  const HostPicker({super.key, required this.hosts, required this.chosen, this.canAdd = true, this.canManage = true});

  final List<HostChoice> hosts;
  final String? chosen;
  final bool canAdd;
  final bool canManage;

  @override
  State<HostPicker> createState() => _HostPickerState();
}

class _HostPickerState extends State<HostPicker> {
  final _search = TextEditingController();

  @override
  void dispose() {
    _search.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final query = _search.text.trim().toLowerCase();
    final shown = [
      for (final host in widget.hosts)
        if (query.isEmpty || host.label.toLowerCase().contains(query) || (host.detail ?? '').toLowerCase().contains(query)) host,
    ];
    return AlertDialog(
      key: const Key('host-picker'),
      title: const Text('Host'),
      content: SizedBox(
        width: 380,
        height: 360,
        child: Column(children: [
          TextField(
            key: const Key('host-search'),
            controller: _search,
            autofocus: true,
            decoration: const InputDecoration(isDense: true, prefixIcon: Icon(Icons.search, size: 18), hintText: 'Search'),
            onChanged: (_) => setState(() {}),
          ),
          const SizedBox(height: 8),
          Expanded(
            child: ListView(children: [
              for (final host in shown)
                ListTile(
                  key: ValueKey('host-choice ${host.name ?? ''}'),
                  dense: true,
                  leading: Icon(host.isLocal ? Icons.computer_outlined : host.kind == ConnectionKind.acp ? Icons.terminal : Icons.dns_outlined),
                  title: Text(host.label, overflow: TextOverflow.ellipsis),
                  subtitle: host.detail == null ? null : Text(host.detail!, overflow: TextOverflow.ellipsis),
                  trailing: host.name == widget.chosen ? const Icon(Icons.check, size: 18) : null,
                  onTap: () => Navigator.pop(context, HostChosen(host.name)),
                ),
              if (shown.isEmpty) const Padding(padding: EdgeInsets.all(12), child: Text('No host matches.')),
            ]),
          ),
        ]),
      ),
      actions: [
        if (widget.canAdd)
          TextButton(key: const Key('host-add'), onPressed: () => Navigator.pop(context, const HostAdd()), child: const Text('Add')),
        if (widget.canManage)
          TextButton(key: const Key('host-manage'), onPressed: () => Navigator.pop(context, const HostManage()), child: const Text('Manage')),
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      ],
    );
  }
}
