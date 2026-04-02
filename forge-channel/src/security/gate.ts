const allowlist = new Set<string>();
const pendingPairings = new Map<string, string>(); // code -> senderId

export function isAllowed(senderId: string): boolean {
  return allowlist.has(senderId);
}

export function addSender(senderId: string): void {
  allowlist.add(senderId);
}

export function generatePairingCode(): string {
  return Math.floor(100000 + Math.random() * 900000).toString();
}

export function startPairing(senderId: string): string {
  const code = generatePairingCode();
  pendingPairings.set(code, senderId);
  // Expire after 5 minutes
  setTimeout(() => pendingPairings.delete(code), 5 * 60 * 1000);
  return code;
}

export function confirmPairing(code: string): boolean {
  const senderId = pendingPairings.get(code);
  if (!senderId) return false;
  addSender(senderId);
  pendingPairings.delete(code);
  return true;
}
