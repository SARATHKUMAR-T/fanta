"""A tiny MCP server over stdio for testing Fanta's client (no dependencies)."""
import json
import sys

tools = [
    {"name": "echo", "description": "Echo text back", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
     "annotations": {"readOnlyHint": True}},
    {"name": "fail", "description": "Always fails", "inputSchema": {"type": "object"}},
    {"name": "add_tool", "description": "Adds a tool and notifies", "inputSchema": {"type": "object"}},
]
ping_answered = False


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    msg = json.loads(line)
    method, mid = msg.get("method"), msg.get("id")
    if method is None:
        # A response to our ping.
        if mid == "srv-ping" and "result" in msg:
            ping_answered = True
        continue
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": msg["params"]["protocolVersion"],
            "capabilities": {"tools": {"listChanged": True}},
            "serverInfo": {"name": "fake-server", "version": "1"}}})
    elif method == "notifications/initialized":
        print("initialized", file=sys.stderr, flush=True)
        send({"jsonrpc": "2.0", "id": "srv-ping", "method": "ping"})
    elif method == "tools/list":
        # Two pages, to exercise pagination.
        cursor = (msg.get("params") or {}).get("cursor")
        if cursor is None:
            send({"jsonrpc": "2.0", "id": mid, "result": {"tools": tools[:1], "nextCursor": "p2"}})
        else:
            send({"jsonrpc": "2.0", "id": mid, "result": {"tools": tools[1:]}})
    elif method == "tools/call":
        name, args = msg["params"]["name"], msg["params"].get("arguments", {})
        if name == "echo":
            text = f"{args.get('text', '')} (ping answered: {ping_answered})"
            send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": text}]}})
        elif name == "fail":
            send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": "boom"}], "isError": True}})
        elif name == "add_tool":
            tools.append({"name": "new_tool", "description": "Added later", "inputSchema": {"type": "object"}})
            send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": "added"}]}})
            send({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})
        elif name == "crash":
            sys.exit(3)
        else:
            send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32602, "message": f"unknown tool {name}"}})
