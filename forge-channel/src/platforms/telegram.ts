import { isAllowed, startPairing } from "../security/gate.js";

interface TelegramMessage {
  message_id: number;
  from: { id: number; first_name: string };
  chat: { id: number };
  text?: string;
}

type NotifyFn = (content: string, meta: Record<string, string>) => Promise<void>;

let lastUpdateId = 0;

export async function startPolling(token: string, notify: NotifyFn): Promise<void> {
  const baseUrl = `https://api.telegram.org/bot${token}`;

  while (true) {
    try {
      const res = await fetch(
        `${baseUrl}/getUpdates?offset=${lastUpdateId + 1}&timeout=30`,
        { signal: AbortSignal.timeout(35000) }
      );
      const data = (await res.json()) as { ok: boolean; result: any[] };
      if (!data.ok || !data.result) continue;

      for (const update of data.result) {
        lastUpdateId = update.update_id;
        const msg: TelegramMessage | undefined = update.message;
        if (!msg?.text) continue;

        const senderId = msg.from.id.toString();

        // Pairing flow
        if (msg.text === "/pair" || msg.text.startsWith("/pair")) {
          const code = startPairing(senderId);
          await sendMessage(baseUrl, msg.chat.id,
            `Pairing code: ${code}\nEnter this in your Claude Code session to link this chat.`);
          continue;
        }

        // Gate: only allowlisted senders
        if (!isAllowed(senderId)) {
          await sendMessage(baseUrl, msg.chat.id,
            "Not paired. Send /pair to get a pairing code.");
          continue;
        }

        // Forward to Claude Code session
        await notify(msg.text, {
          chat_id: msg.chat.id.toString(),
          sender: msg.from.first_name,
          sender_id: senderId,
        });
      }
    } catch {
      // Network error or timeout — retry after brief pause
      await new Promise(r => setTimeout(r, 1000));
    }
  }
}

export async function sendReply(token: string, chatId: string, text: string): Promise<void> {
  const baseUrl = `https://api.telegram.org/bot${token}`;
  await sendMessage(baseUrl, parseInt(chatId), text);
}

async function sendMessage(baseUrl: string, chatId: number, text: string): Promise<void> {
  await fetch(`${baseUrl}/sendMessage`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ chat_id: chatId, text }),
  });
}
