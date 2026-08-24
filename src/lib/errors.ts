// Backend rejection helpers shared by the management pages (Task 17/18).
// Tauri serializes the Rust AppError DTO ({code, message, suggested_action})
// as a plain object/string, not an Error — messages arrive already redacted
// by the core and are surfaced verbatim.

export const AGENT_CONFIRMATION_REQUIRED = "integration.agent_confirmation_required";

export interface PageError {
  message: string;
  suggested_action: string | null;
}

export function errorCodeOf(e: unknown): string | null {
  if (typeof e === "string") {
    return e.includes(AGENT_CONFIRMATION_REQUIRED) ? AGENT_CONFIRMATION_REQUIRED : e;
  }
  if (e instanceof Error) {
    return e.message.includes(AGENT_CONFIRMATION_REQUIRED)
      ? AGENT_CONFIRMATION_REQUIRED
      : e.message;
  }
  if (e !== null && typeof e === "object" && "code" in e) {
    return String((e as { code: unknown }).code);
  }
  return null;
}

export function errorOf(e: unknown): PageError {
  if (e !== null && typeof e === "object") {
    const record = e as Record<string, unknown>;
    return {
      message:
        typeof record.message === "string" ? record.message : JSON.stringify(record),
      suggested_action:
        typeof record.suggested_action === "string" ? record.suggested_action : null,
    };
  }
  if (e instanceof Error) {
    return { message: e.message, suggested_action: null };
  }
  return { message: String(e), suggested_action: null };
}
