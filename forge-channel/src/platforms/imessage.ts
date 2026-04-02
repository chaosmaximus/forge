/**
 * iMessage platform adapter (macOS only).
 * Reads messages from ~/Library/Messages/chat.db (SQLite).
 * Replies via AppleScript osascript.
 * Requires Full Disk Access permission on macOS.
 */
import { isAllowed, startPairing } from "../security/gate.js";
import { execSync } from "child_process";
import { existsSync } from "fs";
import { homedir } from "os";
import { join } from "path";

type NotifyFn = (content: string, meta: Record<string, string>) => Promise<void>;

const DB_PATH = join(homedir(), "Library/Messages/chat.db");
let lastRowId = 0;

export async function startPolling(notify: NotifyFn): Promise<void> {
  // Check macOS and Messages.db access
  if (process.platform !== "darwin") {
    console.error("iMessage channel is macOS-only.");
    process.exit(1);
  }
  if (!existsSync(DB_PATH)) {
    console.error(`Messages database not found at ${DB_PATH}. Ensure Full Disk Access is enabled.`);
    process.exit(1);
  }

  // Get the latest row ID to avoid replaying old messages
  try {
    const result = execSync(
      `sqlite3 "${DB_PATH}" "SELECT MAX(ROWID) FROM message;"`,
      { encoding: "utf-8" }
    ).trim();
    lastRowId = parseInt(result) || 0;
  } catch {
    console.error("Failed to read Messages database. Check Full Disk Access permissions.");
    process.exit(1);
  }

  // Poll loop
  while (true) {
    try {
      const query = `
        SELECT m.ROWID, m.text, m.is_from_me, h.id AS sender_id,
               COALESCE(h.id, '') AS handle
        FROM message m
        LEFT JOIN handle h ON m.handle_id = h.ROWID
        WHERE m.ROWID > ${lastRowId} AND m.is_from_me = 0 AND m.text IS NOT NULL
        ORDER BY m.ROWID ASC
        LIMIT 10;
      `;

      const result = execSync(
        `sqlite3 -separator '|||' "${DB_PATH}" "${query.replace(/\n/g, " ")}"`,
        { encoding: "utf-8" }
      ).trim();

      if (result) {
        for (const row of result.split("\n")) {
          const parts = row.split("|||");
          if (parts.length < 5) continue;

          const [rowId, text, _isFromMe, senderId, handle] = parts;
          lastRowId = Math.max(lastRowId, parseInt(rowId) || 0);

          // Pairing flow
          if (text.trim() === "/pair" || text.trim().startsWith("/pair")) {
            const code = startPairing(senderId);
            await sendReply(handle, `Pairing code: ${code}\nEnter this in your Claude Code session.`);
            continue;
          }

          // Gate
          if (!isAllowed(senderId)) {
            await sendReply(handle, "Not paired. Send /pair to get a pairing code.");
            continue;
          }

          // Forward to Claude Code
          await notify(text, {
            sender_id: senderId,
            handle: handle,
            platform: "imessage",
          });
        }
      }
    } catch {
      // DB locked or other transient error — retry
    }

    await new Promise(r => setTimeout(r, 2000)); // Poll every 2s
  }
}

export async function sendReply(handle: string, text: string): Promise<void> {
  // Sanitize text for AppleScript (escape backslashes and quotes)
  const sanitized = text
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\n/g, "\\n");

  const script = `
    tell application "Messages"
      set targetBuddy to buddy "${handle}" of service 1
      send "${sanitized}" to targetBuddy
    end tell
  `;

  try {
    execSync(`osascript -e '${script.replace(/'/g, "'\\''")}'`, {
      timeout: 10000,
    });
  } catch (e) {
    console.error(`Failed to send iMessage to ${handle}:`, e);
  }
}
