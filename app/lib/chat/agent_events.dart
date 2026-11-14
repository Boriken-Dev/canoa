import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ChatLimits;

// What an agent reports beyond its messages that the app's own session state reads. The tool calls, terminals, plan, compactions,
// commands, usage and choices it draws are `coqui_acp_chat`'s, re-exported here for the files that read them.
export 'package:coqui_acp_chat/coqui_acp_chat.dart'
    show
        AgentCommandInfo,
        ChatLimits,
        Choice,
        Compaction,
        CompactionStatus,
        ConfigOption,
        PlanEntry,
        PlanStatus,
        Rendering,
        MessageRole,
        UpdateRecorder,
        attachedNames,
        renderingOf,
        SessionUsage,
        TerminalView,
        ToolBlock,
        ToolCall,
        ToolDiff,
        ToolPatch,
        ToolStatus,
        ToolTerminal,
        ToolText,
        UsageBucket,
        UsageCoverage,
        UsageLimitHit,
        UsageModelSpend,
        UsageRefill,
        UsageReport,
        UsageSpend,
        UsageSupport,
        UsageUnit,
        describeToolDiff,
        describeToolInput,
        parseCommands,
        parseConfigOptions,
        parseModes,
        parsePlan,
        planItems,
        usageCapability,
        usageMethod,
        usageNotification;

/// The bounds on what an agent sends, at their defaults.
const ChatLimits chatLimits = ChatLimits();

String boundedHead(String text, int limit) => text.length > limit ? '${text.substring(0, limit)}\n…' : text;
