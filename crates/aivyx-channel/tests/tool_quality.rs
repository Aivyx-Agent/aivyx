//! Chapter Atlas (AT.3) — metadata quality sweep over the in-process
//! **infrastructure** tools (the substrate tier is swept in `aivyx-core`).
//!
//! Runs `aivyx_core::tools::check_tool_quality` over a representative tool from
//! each infrastructure module (plus `tools.list`): the name/description/schema
//! the LLM relies on must meet the correctness floor. Constructed via each
//! tool's cheap `Default`/`new` so the sweep needs no live stores/dispatchers
//! (the metadata methods don't touch them).

use std::sync::Arc;

use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;

#[test]
fn infrastructure_tools_meet_quality_floor() {
    use aivyx_channel::{
        loop_tool::LoopNextTool, memory_gc_tool::MemoryGcTool, mission_tool::MissionCreateTool,
        reflection_tool::ReflectionProposeTool, reminder_tool::RemindSetTool,
        role_update_tool::RoleUpdateTool, schedule_tool::ScheduleCreateTool,
        tools_list_tool::ToolsListTool, turn_history_tool::TurnHistoryTool,
    };
    use aivyx_memory::InMemoryMemory;

    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(TurnHistoryTool::default()),
        Arc::new(MemoryGcTool::new(Arc::new(InMemoryMemory::new()))),
        Arc::new(RemindSetTool::default()),
        Arc::new(LoopNextTool::default()),
        Arc::new(MissionCreateTool::default()),
        Arc::new(ScheduleCreateTool::default()),
        Arc::new(ReflectionProposeTool::default()),
        Arc::new(RoleUpdateTool::default()),
        Arc::new(ToolsListTool::new(vec![ToolsListTool::self_info()])),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();
    assert!(
        issues.is_empty(),
        "infrastructure tool quality issues:\n  {}",
        issues.join("\n  "),
    );
}
