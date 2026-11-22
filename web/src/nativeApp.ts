/** NativeApp is the iPhone app when this page is shown inside it; null in a browser. There,
 *  voice mode is the app's own, which keeps running with the phone locked. Posting a chat's ID
 *  starts it in that chat, and posting null ends it. */
type NativeApp = { postMessage: (message: { voice: string | null }) => void };

type Webkit = { messageHandlers?: { covalent?: NativeApp } };

export const nativeApp: NativeApp | null =
  (window as { webkit?: Webkit }).webkit?.messageHandlers?.covalent ?? null;

/** VOICE_ENDED is the window event the app sends when its voice mode has ended. Its detail is
 *  the reason, when it ended because something failed. */
export const VOICE_ENDED = "covalent-voice-ended";
