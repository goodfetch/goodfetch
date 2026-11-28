//! MCP tool profiles (ADRs 0011, 0090, 0095, 0099; spec #608): each profile
//! selects a deterministic set of catalog operations, identical in Collie and
//! Foxhound executables. Granting, person-only and SDK-only operations are
//! kept off every profile.

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::{Operation, Product};
pub use goodfetch_protocol::mcp_projection::{
    KEPT_OFF_OPERATIONS, ProfileName, is_mcp_eligible, profile_operations,
};
use rmcp::model::Tool;

use crate::projection::{ProjectedTool, project_operation};

/// A resolved MCP profile ready to serve `tools/list` and dispatch calls.
#[derive(Clone, Debug)]
pub struct ResolvedProfile {
    /// The profile's name (e.g. `"browser"`).
    pub name: String,
    /// Projected tools sorted by tool name for deterministic snapshots.
    tools: Vec<ProjectedTool>,
    /// Lookup from MCP tool name to index in `tools`.
    index: BTreeMap<String, usize>,
}

impl ResolvedProfile {
    /// Builds one of Collie's five profiles for `product`.
    #[must_use]
    pub fn for_product(product: Product, profile: ProfileName) -> Self {
        let ops = profile_operations(product, profile);
        Self::from_operations(profile.as_str().to_owned(), ops)
    }

    /// Builds a profile from an explicit list of operations (used by Foxhound
    /// profiles or fixture servers).
    #[must_use]
    pub fn from_operations(name: String, operations: Vec<&'static Operation>) -> Self {
        let mut tools: Vec<ProjectedTool> = operations.into_iter().map(project_operation).collect();
        tools.sort_by(|a, b| a.tool.name.cmp(&b.tool.name));
        let index = tools
            .iter()
            .enumerate()
            .map(|(i, projected)| (projected.tool.name.to_string(), i))
            .collect();
        Self { name, tools, index }
    }

    /// Returns the MCP tool descriptors exposed by this profile in sorted order.
    #[must_use]
    pub fn list_tools(&self) -> Vec<Tool> {
        self.tools
            .iter()
            .map(|projected| projected.tool.clone())
            .collect()
    }

    /// Looks up a projected tool by its MCP tool name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ProjectedTool> {
        self.index.get(name).map(|&idx| &self.tools[idx])
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn projected_tools(&self) -> &[ProjectedTool] {
        &self.tools
    }
}
