# Context7 MCP Setup
Context7 is an opt-in MCP server that gives the agent live, version-specific library docs.
Adapt the headers block below to match Context7's current authentication method.

Adapt this MCP server definition for the active Claude Code/MCP client configuration:
```json
{
  "servers": {
    "context7": {
      "type": "http",
      "url": "https://mcp.context7.com/mcp",
      "headers": { "CONTEXT7_API_KEY": "${input:context7-api-key}" }
    }
  },
  "inputs": [
    { "id": "context7-api-key", "type": "promptString", "description": "Context7 API key", "password": true }
  ]
}
```
Available tools:
- `resolve-library-id`: map a library name plus task context to a Context7 library ID.
- `query-docs`: fetch docs for that exact library ID and question.
Usage pattern:
1. Call `resolve-library-id` first.
2. Call `query-docs` with the returned ID.
3. If Context7 is unavailable, fall back to official docs, docs.rs, or crates.io.