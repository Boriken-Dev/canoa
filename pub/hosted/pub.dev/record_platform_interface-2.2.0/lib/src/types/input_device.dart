import 'input_device_type.dart';

class InputDevice {
  /// The ID used to select the device on the platform.
  final String id;

  /// The label text representation.
  final String label;

  /// The transport type of this device.
  ///
  /// Defaults to [InputDeviceType.unknown] on platforms that do not report
  /// device types (Linux, Web).
  final InputDeviceType type;

  /// The sample rates supported or prefered by this device.
  ///
  /// Empty if the platform does not report per-device sample rates (iOS, Web).
  final List<int> sampleRates;

  const InputDevice({
    required this.id,
    required this.label,
    this.type = InputDeviceType.unknown,
    this.sampleRates = const [],
  });

  factory InputDevice.fromMap(Map map) => InputDevice(
    id: map['id'],
    label: map['label'],
    type: InputDeviceType.fromString(map['type'] as String?),
    sampleRates: List<int>.from(map['sampleRates'] as List? ?? const []),
  );

  Map<String, dynamic> toMap() => {
    'id': id,
    'label': label,
    'type': type.name,
    if (sampleRates.isNotEmpty) 'sampleRates': sampleRates,
  };

  @override
  String toString() {
    return '''
      id: $id
      label: $label
      type: $type
      sampleRates: $sampleRates
      ''';
  }

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;

    return other is InputDevice && other.id == id && other.label == label;
  }

  @override
  int get hashCode => id.hashCode ^ label.hashCode;
}
