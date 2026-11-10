#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Loopback-only interop fixture using the official MCP Python SDK (mcp 1.x)."""
import sys
from mcp.server.fastmcp import FastMCP

server = FastMCP("Nola interoperability fixture", host="127.0.0.1", port=int(sys.argv[2]) if len(sys.argv) > 2 else 8000)


@server.tool()
def echo(value: str) -> str:
    """Return text without external side effects."""
    return value


@server.resource("fixture://greeting")
def greeting() -> str:
    return "你好 Nola resource"


@server.prompt()
def welcome(name: str) -> str:
    return f"Welcome {name}"


if __name__ == "__main__":
    server.run(transport=sys.argv[1] if len(sys.argv) > 1 else "stdio")
