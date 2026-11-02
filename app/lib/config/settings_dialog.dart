import 'package:flutter/material.dart';
import 'canoa_config.dart';
import '../theme/look.dart';

class SettingsDialog extends StatefulWidget {
  const SettingsDialog({super.key, required this.config});
  final CanoaConfig config;
  @override
  State<SettingsDialog> createState() => SettingsDialogState();
}
class SettingsDialogState extends State<SettingsDialog> {
  late CanoaConfig config = widget.config;
  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Canoa settings'),
    content: SizedBox(width: 400, child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        ListTile(
          title: const Text('Look'),
          trailing: DropdownButton<LookId>(
            value: config.look,
            items: [for (final look in LookId.values) DropdownMenuItem(value: look, child: Text(look.label))],
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(look: value));
            },
          ),
        ),
        ListTile(
          title: const Text('Brightness'),
          trailing: DropdownButton<String>(
            value: config.brightness,
            items: const [
              DropdownMenuItem(value: 'dark', child: Text('Dark')),
              DropdownMenuItem(value: 'light', child: Text('Light')),
              DropdownMenuItem(value: 'system', child: Text('Follow the system')),
            ],
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(brightness: value));
            },
          ),
        ),
        SwitchListTile(
          title: const Text('Start hidden in tray'),
          value: config.startHidden,
          onChanged: (v) => setState(() => config = config.copyWith(startHidden: v)),
        ),
        SwitchListTile(
          title: const Text('Close window to tray'),
          value: config.closeToTray,
          onChanged: (v) => setState(() => config = config.copyWith(closeToTray: v)),
        ),
        SwitchListTile(
          title: const Text('Minimize window to tray'),
          value: config.minimizeToTray,
          onChanged: (v) => setState(() => config = config.copyWith(minimizeToTray: v)),
        ),
        SwitchListTile(
          title: const Text('Prefer WebTransport (WSS fallback)'),
          value: config.preferWebTransport,
          onChanged: (v) => setState(() =>
              config = config.copyWith(preferWebTransport: v)),
        ),
        ListTile(
          title: const Text('Network timeout'),
          subtitle: Text('${config.connectTimeout} seconds'),
          trailing: DropdownButton<int>(
            value: config.connectTimeout,
            items: [5,10,15,30,60].map((n) => DropdownMenuItem(
              value: n, child: Text('$n s'))).toList(),
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(connectTimeout: value));
            },
          ),
        ),
      ],
    )),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(onPressed: () => Navigator.pop(context, config), child: const Text('Save')),
    ],
  );
}
