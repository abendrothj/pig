//! Allowlist of code-graph operations a `CodeIntelligenceProvider` may execute.
//!
//! Deliberately excludes mutating or index-triggering tools the underlying provider may
//! expose (e.g. codebase-memory-mcp's `index_repository`, `delete_project`,
//! `ingest_traces`) and annotation tools (`manage_adr`) — the code graph is read-only
//! derived state, LAO never mutates it and never silently triggers a full index.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphOperation {
    SearchGraph,
    SearchCode,
    TracePath,
    GetCodeSnippet,
    GetArchitecture,
    QueryGraph,
    IndexStatus,
}

impl GraphOperation {
    pub const ALL: &'static [GraphOperation] = &[
        GraphOperation::SearchGraph,
        GraphOperation::SearchCode,
        GraphOperation::TracePath,
        GraphOperation::GetCodeSnippet,
        GraphOperation::GetArchitecture,
        GraphOperation::QueryGraph,
        GraphOperation::IndexStatus,
    ];

    pub fn tool_name(&self) -> &'static str {
        match self {
            GraphOperation::SearchGraph => "search_graph",
            GraphOperation::SearchCode => "search_code",
            GraphOperation::TracePath => "trace_path",
            GraphOperation::GetCodeSnippet => "get_code_snippet",
            GraphOperation::GetArchitecture => "get_architecture",
            GraphOperation::QueryGraph => "query_graph",
            GraphOperation::IndexStatus => "index_status",
        }
    }

    /// Reject anything not in the allowlist before a provider process is ever spawned.
    pub fn from_tool_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|op| op.tool_name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_round_trips_through_its_tool_name() {
        for op in GraphOperation::ALL {
            assert_eq!(GraphOperation::from_tool_name(op.tool_name()), Some(*op));
        }
    }

    #[test]
    fn mutating_and_index_triggering_tools_are_not_in_the_allowlist() {
        for forbidden in [
            "index_repository",
            "delete_project",
            "manage_adr",
            "ingest_traces",
        ] {
            assert_eq!(GraphOperation::from_tool_name(forbidden), None);
        }
    }

    #[test]
    fn unknown_operation_name_is_rejected() {
        assert_eq!(GraphOperation::from_tool_name("not_a_real_tool"), None);
    }
}
