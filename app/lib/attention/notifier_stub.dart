import 'notifier.dart';

Notifier createNotifier() => const _None();

/// A platform with no way to notify: nothing is shown.
final class _None implements Notifier {
  const _None();

  @override
  Future<void> initialize(void Function(String sessionId) onOpen) async {}

  @override
  Future<bool> get available async => false;

  @override
  Future<bool> requestPermission() async => false;

  @override
  void watchAttention(void Function(bool attended) onChange) => onChange(false);

  @override
  Future<void> show({required String title, required String body, required String sessionId}) async {}
}
