import '../providers/provider_catalog.dart';
import 'look.dart';
import 'vendor_looks.dart';

/// The family the app draws in. With [follow] on and a conversation shown, its agent's vendor picks it, among the families the host
/// lists ([vendorLooks]); any other agent, a vendor the host lists none for, and every section but the conversations keep [chosen].
LookId familyFor({required LookId chosen, required bool follow, required bool inConversations, String? provider}) {
  if (!follow || !inConversations) return chosen;
  final vendor = providerCatalog.resolve(provider)?.id;
  return vendor == null ? chosen : vendorLooks.familyOfVendor(vendor) ?? chosen;
}
