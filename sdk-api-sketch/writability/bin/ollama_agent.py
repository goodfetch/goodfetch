#!/usr/bin/env python3
"""A minimal coding agent on a local ollama model, for the writability runs.

Usage: ollama_agent.py RUN_DIR [MODEL]

The model gets the run's prompt.md and five tools (list_dir, read_file, grep, write_file,
run_check), confined to the run folder and the library the prompt names; it may write only the
program. The transcript and a summary are saved to RUN_DIR/.agent/.
"""

import json
import re
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

T = Path(__file__).resolve().parent.parent
OLLAMA = "http://127.0.0.1:11434/api/chat"
NUM_CTX = 32768
MAX_TURNS = 80
MAX_SECONDS = 20 * 60
MAX_CHECKS = 3
CONTEXT_CHARS = 70_000  # about 20k tokens of messages; the oldest tool outputs are elided past it

SYSTEM = (
    "You are a coding agent working in a sandbox. You act only through your tools: list_dir, read_file, "
    "grep, write_file and run_check. Explore the library's files with them, write the program with "
    "write_file, then type-check it with run_check. Follow the task's 'How to work' section. When you "
    "are finished, reply with the final line it asks for and no tool call."
)


def fn(name: str, description: str, props: dict, required: list[str]) -> dict:
    return {
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {"type": "object", "properties": props, "required": required},
        },
    }


S = {"type": "string"}
I = {"type": "integer"}
TOOLS = [
    fn("list_dir", "List the files under a folder, recursively (at most 300 entries).", {"path": S}, ["path"]),
    fn(
        "read_file",
        "Read a text file with line numbers, at most 300 lines per call; read on with offset.",
        {"path": S, "offset": I, "limit": I},
        ["path"],
    ),
    fn("grep", "Search files under a folder for an extended regular expression (at most 100 matching lines).", {"pattern": S, "path": S}, ["pattern", "path"]),
    fn("write_file", "Write the whole content of the program file, replacing it.", {"path": S, "content": S}, ["path", "content"]),
    fn("run_check", "Type-check the program (at most 3 times in all). The first run records your first attempt.", {}, []),
]


def chat(model: str, messages: list[dict]) -> dict:
    body = json.dumps({"model": model, "messages": messages, "tools": TOOLS, "stream": False, "options": {"num_ctx": NUM_CTX}})
    req = urllib.request.Request(OLLAMA, data=body.encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=1800) as resp:
        return json.loads(resp.read())


def read_roots(rundir: Path, task: dict) -> list[Path]:
    roots = [rundir.resolve()]
    if task["kind"] == "gf":
        roots.append((T / "sketch").resolve())
    elif task["lang"] == "ts":
        roots.append((T / task["env"] / "node_modules").resolve())
    else:
        roots.append((T / "env/py" / task["env"] / "lib/python3.11/site-packages").resolve())
    return roots


def inside(path: Path, roots: list[Path]) -> bool:
    try:
        rp = path.resolve()
    except OSError:
        return False
    return any(rp == r or r in rp.parents for r in roots)


def list_dir(args: dict, roots: list[Path]) -> str:
    p = Path(str(args.get("path", "")))
    if not inside(p, roots) or not p.is_dir():
        return "error: not a folder you may read"
    out = []
    for q in sorted(p.rglob("*")):
        rel = q.relative_to(p).parts
        if any(part in ("node_modules", ".checks", ".agent", "__pycache__") for part in rel[:-1]):
            continue
        out.append("/".join(rel) + ("/" if q.is_dir() else ""))
        if len(out) >= 300:
            out.append("... (truncated)")
            break
    return "\n".join(out)


def read_file(args: dict, roots: list[Path]) -> str:
    p = Path(str(args.get("path", "")))
    if not inside(p, roots) or not p.is_file():
        return "error: not a file you may read"
    lines = p.read_text(errors="replace").splitlines()
    offset = max(1, int(args.get("offset") or 1))
    limit = min(300, max(1, int(args.get("limit") or 300)))
    text = "\n".join(f"{offset + i}\t{l}" for i, l in enumerate(lines[offset - 1 : offset - 1 + limit]))
    if offset - 1 + limit < len(lines):
        text += f"\n... ({len(lines)} lines in all; read on with offset={offset + limit})"
    return text


def grep(args: dict, roots: list[Path]) -> str:
    p = Path(str(args.get("path", "")))
    if not inside(p, roots):
        return "error: not a folder you may read"
    try:
        proc = subprocess.run(
            ["grep", "-rnE", "--exclude-dir=.checks", "--exclude-dir=.agent", str(args.get("pattern", "")), str(p)],
            capture_output=True,
            text=True,
            timeout=60,
        )
    except subprocess.TimeoutExpired:
        return "error: grep timed out"
    lines = proc.stdout.splitlines()
    if len(lines) > 100:
        lines = lines[:100] + [f"... ({len(lines) - 100} more matches)"]
    return "\n".join(l[:400] for l in lines) or "no match"


def calls_in_text(text: str) -> list[dict]:
    """Tool calls a model wrote as text (qwen2.5-coder through ollama writes them as JSON in a code
    fence or in <tool_call> tags instead of the API's tool_calls)."""
    found = []
    blocks = re.findall(r"<tool_call>\s*(.*?)\s*</tool_call>", text, re.S)
    blocks += re.findall(r"```(?:json)?\s*(\{.*?\})\s*```", text, re.S)
    if not blocks and text.strip().startswith("{"):
        blocks = [text.strip()]
    for b in blocks:
        try:
            obj = json.loads(b)
        except json.JSONDecodeError:
            continue
        if isinstance(obj, dict) and isinstance(obj.get("name"), str):
            found.append({"function": {"name": obj["name"], "arguments": obj.get("arguments") or obj.get("parameters") or {}}})
    return found


def main() -> None:
    rundir = Path(sys.argv[1]).resolve()
    model = sys.argv[2] if len(sys.argv) > 2 else "qwen2.5-coder:14b"
    task = json.loads((rundir / "task.json").read_text())
    program = (rundir / task["program"]).resolve()
    roots = read_roots(rundir, task)
    prompt = (rundir / "prompt.md").read_text().replace(f"by running `{rundir / 'check'}`", "with the run_check tool")
    messages: list[dict] = [{"role": "system", "content": SYSTEM}, {"role": "user", "content": prompt}]
    agent_dir = rundir / ".agent"
    agent_dir.mkdir(exist_ok=True)
    checks, started, end, turn = 0, time.time(), "turns", 0
    stats = {"prompt_tokens": 0, "output_tokens": 0}
    for turn in range(1, MAX_TURNS + 1):
        if time.time() - started > MAX_SECONDS:
            end = "time"
            break
        total = sum(len(m.get("content") or "") for m in messages)
        for m in messages:
            if total <= CONTEXT_CHARS:
                break
            if m["role"] == "tool" and not m["content"].startswith("[elided"):
                total -= len(m["content"])
                m["content"] = "[elided to save context; call the tool again if you need it]"
        try:
            resp = chat(model, messages)
        except Exception as e:  # noqa: BLE001 - recorded, and it ends the run
            end = f"error: {e}"
            break
        stats["prompt_tokens"] += resp.get("prompt_eval_count", 0)
        stats["output_tokens"] += resp.get("eval_count", 0)
        msg = resp["message"]
        messages.append({k: v for k, v in msg.items() if k in ("role", "content", "tool_calls")})
        calls = msg.get("tool_calls") or calls_in_text(msg.get("content") or "")
        if calls and not msg.get("tool_calls"):
            stats["text_tool_calls"] = stats.get("text_tool_calls", 0) + len(calls)
        if not calls:
            end = "replied"
            break
        for call in calls:
            name = call["function"]["name"]
            args = call["function"].get("arguments") or {}
            if isinstance(args, str):
                try:
                    args = json.loads(args)
                except json.JSONDecodeError:
                    args = {}
            if name == "list_dir":
                out = list_dir(args, roots)
            elif name == "read_file":
                out = read_file(args, roots)
            elif name == "grep":
                out = grep(args, roots)
            elif name == "write_file":
                target = Path(str(args.get("path") or program)).resolve()
                if target != program:
                    out = f"error: you may only write {program}"
                else:
                    program.write_text(str(args.get("content", "")))
                    out = f"wrote {len(str(args.get('content', '')))} characters to {program}"
            elif name == "run_check":
                if checks >= MAX_CHECKS:
                    out = "error: no check runs left (3 used); reply with your final line"
                else:
                    checks += 1
                    proc = subprocess.run([sys.executable, str(T / "bin/check.py"), str(rundir)], capture_output=True, text=True)
                    out = (proc.stdout + proc.stderr)[-12000:]
            else:
                out = f"error: unknown tool {name}"
            messages.append({"role": "tool", "content": out, "tool_name": name})
        (agent_dir / "transcript.json").write_text(json.dumps(messages, indent=1))
    result = {
        "model": model,
        "num_ctx": NUM_CTX,
        "turns": turn,
        "checks": checks,
        "end": end,
        "seconds": round(time.time() - started),
        **stats,
        "final_text": (messages[-1].get("content") or "")[-500:],
    }
    (agent_dir / "transcript.json").write_text(json.dumps(messages, indent=1))
    (agent_dir / "result.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result))


if __name__ == "__main__":
    main()
