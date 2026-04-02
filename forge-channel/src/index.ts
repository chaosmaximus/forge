import { createServer } from "./server.js";

const platform = process.argv[2] || "telegram";
const token = process.env.FORGE_CHANNEL_TOKEN || "";

if (platform === "telegram" && !token) {
  console.error("Set FORGE_CHANNEL_TOKEN env var for Telegram");
  process.exit(1);
}

await createServer(platform, token);
