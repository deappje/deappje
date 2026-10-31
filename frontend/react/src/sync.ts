import { useCallback, useEffect, useRef, useState } from "react";
import { type Client, describeError, type SyncReport } from "./core";

/** What the periodic sync tells the screens. */
export interface SyncState {
  /** Bumps after every sync: a screen re-reads the core on it. */
  tick: number;
  syncing: boolean;
  last: SyncReport | undefined;
  error: string | undefined;
  /** One sync now (after a write, after an approval). */
  syncNow: () => Promise<void>;
}

/**
 * Syncs on load, when the tab comes back into view or focus, and every 30
 * seconds while visible. The browser gets no push, so this is its whole
 * wake-up (core/AGENTS.md, "Wake-ups"); the core's lock serialises a sync
 * that overlaps a write, and `running` keeps a slow one from stacking.
 */
export function useSync(client: Client | undefined, enabled: boolean, periodMs = 30_000): SyncState {
  const [tick, setTick] = useState(0);
  const [syncing, setSyncing] = useState(false);
  const [last, setLast] = useState<SyncReport | undefined>();
  const [error, setError] = useState<string | undefined>();
  const running = useRef(false);

  const syncNow = useCallback(async () => {
    if (!client || !enabled || running.current) return;
    running.current = true;
    setSyncing(true);
    try {
      const report = await client.core.sync();
      setLast(report);
      setError(report.failed.length > 0 ? report.failed.join("; ") : undefined);
    } catch (e) {
      setError(describeError(e));
    } finally {
      running.current = false;
      setSyncing(false);
      setTick((t) => t + 1);
    }
  }, [client, enabled]);

  useEffect(() => {
    if (!client || !enabled) return;
    void syncNow();
    const whenVisible = () => {
      if (document.visibilityState === "visible") void syncNow();
    };
    window.addEventListener("focus", whenVisible);
    document.addEventListener("visibilitychange", whenVisible);
    const timer = setInterval(whenVisible, periodMs);
    return () => {
      window.removeEventListener("focus", whenVisible);
      document.removeEventListener("visibilitychange", whenVisible);
      clearInterval(timer);
    };
  }, [client, enabled, periodMs, syncNow]);

  return { tick, syncing, last, error, syncNow };
}
