//! Generates Collie's MCP tool projection table for TypeScript Framework
//! adapters (ADRs 0011, 0070, 0095, 0099; spec #608).
//!
//! Each tool's JSON Schema is emitted once in `COLLIE_MCP_TOOLS` and each
//! profile maps to its sorted tool names so adapters share one canonical,
//! compact projection matching `goodfetch-mcp`.

use std::fmt::Write as _;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::{OperationClass, Product};
use goodfetch_protocol::mcp_projection::{
    KEPT_OFF_OPERATIONS, McpToolSpec, ProfileName, profile_tools,
};

use super::HEADER;

pub(super) fn file() -> Result<String> {
    let mut out = String::from(HEADER);
    out.push_str(
        "\n\
         /** Collie's five MCP and Framework adapter tool profiles (ADRs 0011, 0095; spec #608). */\n\
         export type McpProfile = \"browser\" | \"agent\" | \"run\" | \"code\" | \"full\";\n\
         \n\
         /** The catalog class of a projected tool. */\n\
         export type McpOperationClass = \"read\" | \"idempotent_write\" | \"effect\";\n\
         \n\
         /** One catalog operation projected onto its MCP and Framework adapter tool shape. */\n\
         export interface McpToolSpec {\n\
           readonly name: string;\n\
           readonly operation: string;\n\
           readonly description: string;\n\
           readonly class: McpOperationClass;\n\
           readonly readOnly: boolean;\n\
           readonly destructive: boolean;\n\
           readonly idempotent: boolean;\n\
           readonly openWorld: boolean;\n\
           readonly cancellable: boolean;\n\
           readonly inputSchema: Readonly<Record<string, unknown>>;\n\
         }\n\n",
    );

    let kept_off_json =
        serde_json::to_string(KEPT_OFF_OPERATIONS).context("serializing KEPT_OFF_OPERATIONS")?;
    writeln!(
        out,
        "/** Operations kept off every MCP profile and Framework adapter (ADR 0011, ADR 0070). */\n\
         export const KEPT_OFF_OPERATIONS: readonly string[] = {kept_off_json};\n"
    )?;

    // `full` contains every eligible Collie operation; store each spec once.
    let all_tools = profile_tools(Product::Collie, ProfileName::Full);
    out.push_str(
        "/** Every eligible Collie tool, keyed by its MCP tool name. */\n\
         export const COLLIE_MCP_TOOLS: Readonly<Record<string, McpToolSpec>> = {\n",
    );
    for spec in &all_tools {
        write_tool_entry(&mut out, spec)?;
    }
    out.push_str("};\n\n");

    out.push_str(
        "/** Sorted MCP tool names in each Collie profile. */\n\
         export const COLLIE_MCP_PROFILES: Readonly<Record<McpProfile, readonly string[]>> = {\n",
    );
    for profile in ProfileName::ALL {
        let names: Vec<String> = profile_tools(Product::Collie, profile)
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        let names_json = serde_json::to_string(&names).context("serializing profile tool names")?;
        writeln!(out, "  {}: {names_json},", profile.as_str())?;
    }
    out.push_str("};\n\n");

    out.push_str(
        "/** Returns the projected Collie tools for `profile` (default `browser`). */\n\
         export function collieMcpTools(profile: McpProfile = \"browser\"): readonly McpToolSpec[] {\n\
           const names = COLLIE_MCP_PROFILES[profile];\n\
           const result: McpToolSpec[] = [];\n\
           for (const name of names) {\n\
             const spec = COLLIE_MCP_TOOLS[name];\n\
             if (spec !== undefined) {\n\
               result.push(spec);\n\
             }\n\
           }\n\
           return result;\n\
         }\n",
    );

    Ok(out)
}

fn write_tool_entry(out: &mut String, spec: &McpToolSpec) -> Result<()> {
    let class_str = match spec.class {
        OperationClass::Read => "read",
        OperationClass::IdempotentWrite => "idempotent_write",
        OperationClass::Effect => "effect",
    };
    let name = serde_json::to_string(&spec.name).context("serializing tool name")?;
    let operation = serde_json::to_string(&spec.operation).context("serializing operation")?;
    let description =
        serde_json::to_string(&spec.description).context("serializing tool description")?;
    let schema = serde_json::to_string(&spec.input_schema).context("serializing input schema")?;
    writeln!(
        out,
        "  {name}: {{\n\
           name: {name},\n\
           operation: {operation},\n\
           description: {description},\n\
           class: \"{class_str}\",\n\
           readOnly: {},\n\
           destructive: {},\n\
           idempotent: {},\n\
           openWorld: {},\n\
           cancellable: {},\n\
           inputSchema: {schema},\n\
         }},",
        spec.read_only(),
        spec.destructive(),
        spec.idempotent(),
        spec.open_world,
        spec.cancellable
    )?;
    Ok(())
}
