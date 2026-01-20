#!/usr/bin/env python3
from __future__ import annotations

import argparse
import sys
from pathlib import Path

from dotenv import load_dotenv
from agents import Agent, ModelSettings, Runner
from agents.mcp import MCPServerStdio


def read_prompt(args: argparse.Namespace) -> str:
    if args.prompt_file:
        return Path(args.prompt_file).read_text()
    if args.prompt:
        return args.prompt
    return sys.stdin.read()


def main() -> int:
    parser = argparse.ArgumentParser(description="Run Ralph via Agents SDK.")
    parser.add_argument("--prompt-file", dest="prompt_file")
    parser.add_argument("--prompt")
    parser.add_argument("--model", default="gpt-5.2-codex")
    parser.add_argument("--max-turns", type=int, default=24)
    parser.add_argument("--reasoning-effort", default="xhigh")
    parser.add_argument("--specialization")
    args = parser.parse_args()

    load_dotenv()
    prompt = read_prompt(args)
    if args.specialization:
        prompt = f"[Specialization]\n{args.specialization}\n\n{prompt}"

    codex_server = MCPServerStdio(
        {
            "command": "npx",
            "args": ["-y", "codex", "mcp-server"],
        },
        name="codex",
    )

    model_settings = ModelSettings(
        metadata={"specialization": args.specialization} if args.specialization else None
    )

    agent = Agent(
        name="Ralph",
        instructions=prompt,
        model=args.model,
        output_type=str,
        mcp_servers=[codex_server],
        model_settings=model_settings,
    )

    result = Runner.run_sync(agent, input="Begin.", max_turns=args.max_turns)
    output = result.final_output_as(str)
    sys.stdout.write(output)
    if not output.endswith("\n"):
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
