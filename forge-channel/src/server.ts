import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  ListToolsRequestSchema,
  CallToolRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import * as telegram from "./platforms/telegram.js";
import * as imessage from "./platforms/imessage.js";
import { confirmPairing } from "./security/gate.js";

export async function createServer(platform: string, token: string) {
  const server = new Server(
    { name: `forge-channel-${platform}`, version: "0.2.0" },
    {
      capabilities: {
        experimental: {
          "claude/channel": {},
        },
        tools: {},
      },
      instructions:
        `You are receiving messages from ${platform} via the Forge channel. ` +
        `When you receive a <channel> event, the sender is authenticated and allowlisted. ` +
        `To reply, call the 'reply' tool with the chat_id from the event metadata and your response text. ` +
        `Keep replies concise — they go to a mobile chat interface.`,
    }
  );

  // Tool definitions
  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: [
      {
        name: "reply",
        description: `Send a reply back through ${platform}`,
        inputSchema: {
          type: "object" as const,
          properties: {
            chat_id: { type: "string", description: "Chat ID from event meta" },
            text: { type: "string", description: "Message text" },
          },
          required: ["chat_id", "text"],
        },
      },
      {
        name: "pair",
        description: "Confirm a pairing code to link a sender",
        inputSchema: {
          type: "object" as const,
          properties: {
            code: { type: "string", description: "6-digit pairing code" },
          },
          required: ["code"],
        },
      },
    ],
  }));

  // Tool handlers
  server.setRequestHandler(CallToolRequestSchema, async (req) => {
    const args = req.params.arguments as Record<string, string>;

    if (req.params.name === "reply") {
      if (platform === "telegram") {
        await telegram.sendReply(token, args.chat_id, args.text);
      } else if (platform === "imessage") {
        await imessage.sendReply(args.chat_id, args.text); // chat_id is the handle
      }
      return { content: [{ type: "text" as const, text: "Sent." }] };
    }

    if (req.params.name === "pair") {
      const ok = confirmPairing(args.code);
      return {
        content: [{ type: "text" as const, text: ok ? "Paired successfully." : "Invalid or expired code." }],
      };
    }

    throw new Error(`Unknown tool: ${req.params.name}`);
  });

  // Start platform polling
  const notify = async (content: string, meta: Record<string, string>) => {
    await server.notification({
      method: "notifications/claude/channel",
      params: { content, meta },
    });
  };

  if (platform === "telegram") {
    telegram.startPolling(token, notify);
  }

  if (platform === "imessage") {
    imessage.startPolling(notify);
  }

  // Connect over stdio
  const transport = new StdioServerTransport();
  await server.connect(transport);
}
