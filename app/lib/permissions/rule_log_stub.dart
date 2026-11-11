import 'permission_rules.dart' show RuleLog;

/// A browser keeps no log: it has no file to append to.
RuleLog? platformRuleLog({String? configPath}) => null;
