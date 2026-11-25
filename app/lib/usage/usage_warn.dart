import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show defaultWarnPermille;
import 'package:material_ui/material_ui.dart';

/// `[usage] warn_permille` for every usage view below it: the composer's pill and the Accounts page read it from here, so a
/// view cannot show another threshold than the one in Settings.
class UsageWarn extends InheritedWidget {
  const UsageWarn({super.key, required this.permille, required super.child});

  /// Thousandths used from which a bucket is shown as running low.
  final int permille;

  /// The threshold in force at [context], the default when no [UsageWarn] is above it.
  static int of(BuildContext context) => context.dependOnInheritedWidgetOfExactType<UsageWarn>()?.permille ?? defaultWarnPermille;

  @override
  bool updateShouldNotify(UsageWarn old) => permille != old.permille;
}
