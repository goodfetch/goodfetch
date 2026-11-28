//! Generates Collie's MCP tool projection table for Python Framework adapters
//! (ADRs 0011, 0070, 0095, 0099; spec #608).
//!
//! Each tool's JSON Schema is emitted once in `COLLIE_MCP_TOOLS` and each
//! profile maps to its sorted tool names so Python adapters share one
//! canonical, compact projection matching `goodfetch-mcp` and TypeScript.

use std::fmt::Write as _;

use anyhow::Result;
use goodfetch_protocol::catalog::{OperationClass, Product};
use goodfetch_protocol::mcp_projection::{
    KEPT_OFF_OPERATIONS, McpToolSpec, ProfileName, profile_tools,
};
use serde_json::Value;

use super::header;

pub(super) fn file() -> Result<String> {
    let mut out = header(
        "Collie's MCP and Framework adapter tool projection table (ADRs 0011, 0070, 0095, 0099).",
    );
    out.push_str(
        "\n\
         from dataclasses import dataclass\n\
         from typing import Any, Literal, TypeAlias\n\
         \n\
         __all__ = [\n\
         \x20   \"COLLIE_MCP_PROFILES\",\n\
         \x20   \"COLLIE_MCP_TOOLS\",\n\
         \x20   \"KEPT_OFF_OPERATIONS\",\n\
         \x20   \"McpOperationClass\",\n\
         \x20   \"McpProfile\",\n\
         \x20   \"McpToolSpec\",\n\
         \x20   \"collie_mcp_tools\",\n\
         ]\n\
         \n\
         McpProfile: TypeAlias = Literal[\"browser\", \"agent\", \"run\", \"code\", \"full\"]\n\
         \"\"\"Collie's five MCP and Framework adapter tool profiles (ADRs 0011, 0095).\"\"\"\n\
         \n\
         McpOperationClass: TypeAlias = Literal[\"read\", \"idempotent_write\", \"effect\"]\n\
         \"\"\"The catalog class of a projected tool.\"\"\"\n\
         \n\
         \n\
         @dataclass(frozen=True)\n\
         class McpToolSpec:\n\
         \x20   \"\"\"One catalog operation projected onto its MCP and Framework adapter tool shape.\"\"\"\n\
         \n\
         \x20   name: str\n\
         \x20   operation: str\n\
         \x20   description: str\n\
         \x20   operation_class: McpOperationClass\n\
         \x20   read_only: bool\n\
         \x20   destructive: bool\n\
         \x20   idempotent: bool\n\
         \x20   open_world: bool\n\
         \x20   cancellable: bool\n\
         \x20   input_schema: dict[str, Any]\n\
         \n\
         \n\
         KEPT_OFF_OPERATIONS: tuple[str, ...] = (\n",
    );
    for op in KEPT_OFF_OPERATIONS {
        writeln!(out, "    {},", py_str(op))?;
    }
    out.push_str(
        ")\n\
         \"\"\"Operations kept off every MCP profile and Framework adapter (ADR 0011, ADR 0070).\"\"\"\n\
         \n\
         COLLIE_MCP_TOOLS: dict[str, McpToolSpec] = {\n",
    );

    let all_tools = profile_tools(Product::Collie, ProfileName::Full);
    for spec in &all_tools {
        write_tool_entry(&mut out, spec)?;
    }
    out.push_str(
        "}\n\
         \"\"\"Every eligible Collie tool, keyed by its MCP tool name.\"\"\"\n\
         \n\
         COLLIE_MCP_PROFILES: dict[McpProfile, tuple[str, ...]] = {\n",
    );

    for profile in ProfileName::ALL {
        let names: Vec<String> = profile_tools(Product::Collie, profile)
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        writeln!(out, "    {}: (", py_str(profile.as_str()))?;
        for name in &names {
            writeln!(out, "        {},", py_str(name))?;
        }
        out.push_str("    ),\n");
    }
    out.push_str(
        "}\n\
         \"\"\"Sorted MCP tool names in each Collie profile.\"\"\"\n\
         \n\
         \n\
         def collie_mcp_tools(profile: McpProfile = \"browser\") -> tuple[McpToolSpec, ...]:\n\
         \x20   \"\"\"Returns the projected Collie tools for `profile` (default `browser`).\"\"\"\n\
         \x20   return tuple(COLLIE_MCP_TOOLS[name] for name in COLLIE_MCP_PROFILES[profile])\n",
    );

    Ok(out)
}

fn write_tool_entry(out: &mut String, spec: &McpToolSpec) -> Result<()> {
    let class_str = match spec.class {
        OperationClass::Read => "read",
        OperationClass::IdempotentWrite => "idempotent_write",
        OperationClass::Effect => "effect",
    };
    write!(
        out,
        "    {name}: McpToolSpec(\n\
         \x20       name={name},\n\
         \x20       operation={op},\n\
         \x20       description={desc},\n\
         \x20       operation_class={cls},\n\
         \x20       read_only={ro},\n\
         \x20       destructive={dest},\n\
         \x20       idempotent={idem},\n\
         \x20       open_world={ow},\n\
         \x20       cancellable={canc},\n\
         \x20       input_schema=",
        name = py_str(&spec.name),
        op = py_str(spec.operation),
        desc = py_str(&spec.description),
        cls = py_str(class_str),
        ro = py_bool(spec.read_only()),
        dest = py_bool(spec.destructive()),
        idem = py_bool(spec.idempotent()),
        ow = py_bool(spec.open_world),
        canc = py_bool(spec.cancellable),
    )?;
    write_py_object(out, &spec.input_schema)?;
    out.push_str(",\n    ),\n");
    Ok(())
}

fn py_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn py_str(value: &str) -> String {
    serde_json::to_string(value).expect("invariant: string always serializes as JSON string")
}

fn write_py_object(out: &mut String, entries: &serde_json::Map<String, Value>) -> Result<()> {
    out.push('{');
    for (index, (key, item)) in entries.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&py_str(key));
        out.push_str(": ");
        write_py_value(out, item)?;
    }
    out.push('}');
    Ok(())
}

fn write_py_value(out: &mut String, value: &Value) -> Result<()> {
    match value {
        Value::Null => out.push_str("None"),
        Value::Bool(flag) => out.push_str(py_bool(*flag)),
        Value::Number(num) => write!(out, "{num}")?,
        Value::String(text) => out.push_str(&py_str(text)),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_py_value(out, item)?;
            }
            out.push(']');
        }
        Value::Object(entries) => write_py_object(out, entries)?,
    }
    Ok(())
}
