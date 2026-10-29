import React from 'react';
import { message } from 'antd';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  cancelCodexOfficialAccountDeviceAuth,
  startCodexOfficialAccountDeviceAuth,
  type CodexDeviceAuthStartResult,
} from '@/services/codexApi';

interface CodexAuthStatusEvent {
  sessionId: string;
  status: string;
  message?: string;
  accountId?: string;
}

export interface CodexDeviceAuthView {
  providerId: string;
  session: CodexDeviceAuthStartResult | null;
  status: string;
  error: string | null;
}

interface DeviceAuthRun {
  providerId: string;
  session: CodexDeviceAuthStartResult | null;
  unlisten: UnlistenFn | null;
  earlyEvents: CodexAuthStatusEvent[];
  cancelled: boolean;
  terminal: boolean;
}

export function useCodexDeviceAuth(onCompleted: () => Promise<void>) {
  const [view, setView] = React.useState<CodexDeviceAuthView | null>(null);
  const runRef = React.useRef<DeviceAuthRun | null>(null);
  const onCompletedRef = React.useRef(onCompleted);
  onCompletedRef.current = onCompleted;

  const close = React.useCallback(() => {
    const run = runRef.current;
    runRef.current = null;
    setView(null);
    if (!run) return;
    run.cancelled = true;
    run.unlisten?.();
    if (run.session && !run.terminal) {
      void cancelCodexOfficialAccountDeviceAuth(run.session.sessionId).catch((error) => {
        console.error('Failed to cancel Codex device auth:', error);
      });
    }
  }, []);

  React.useEffect(() => () => {
    const run = runRef.current;
    runRef.current = null;
    if (!run) return;
    run.cancelled = true;
    run.unlisten?.();
    if (run.session && !run.terminal) {
      void cancelCodexOfficialAccountDeviceAuth(run.session.sessionId).catch(() => undefined);
    }
  }, []);

  const handleStatus = React.useCallback((run: DeviceAuthRun, event: CodexAuthStatusEvent) => {
    if (runRef.current !== run || run.cancelled || run.terminal) return;
    if (!run.session) {
      run.earlyEvents.push(event);
      return;
    }
    if (event.sessionId !== run.session.sessionId) return;
    if (event.status === 'waiting_for_user' || event.status === 'exchanging') {
      setView((current) => current && current.session?.sessionId === run.session?.sessionId
        ? { ...current, status: event.status, error: null } : current);
      return;
    }
    if (!['authorized', 'cancelled', 'expired', 'error'].includes(event.status)) return;
    run.terminal = true;
    run.unlisten?.();
    run.unlisten = null;
    if (event.status === 'authorized') {
      runRef.current = null;
      setView(null);
      void onCompletedRef.current().catch((error) => {
        console.error('Failed to reload Codex after device auth:', error);
        message.error(error instanceof Error ? error.message : String(error));
      });
    } else if (event.status === 'cancelled') {
      runRef.current = null;
      setView(null);
    } else {
      setView((current) => current && current.session?.sessionId === run.session?.sessionId
        ? { ...current, status: event.status, error: event.message ?? null } : current);
    }
  }, []);

  const start = React.useCallback(async (providerId: string) => {
    close();
    const run: DeviceAuthRun = {
      providerId,
      session: null,
      unlisten: null,
      earlyEvents: [],
      cancelled: false,
      terminal: false,
    };
    runRef.current = run;
    setView({ providerId, session: null, status: 'starting', error: null });
    try {
      // The device poll may emit a terminal event before start IPC resolves.
      run.unlisten = await listen<CodexAuthStatusEvent>('codex-auth-status', (event) => {
        handleStatus(run, event.payload);
      });
      if (run.cancelled || runRef.current !== run) {
        run.unlisten();
        return;
      }
      const session = await startCodexOfficialAccountDeviceAuth(providerId);
      run.session = session;
      if (run.cancelled || runRef.current !== run) {
        void cancelCodexOfficialAccountDeviceAuth(session.sessionId).catch((error) => {
          console.error('Failed to cancel Codex device auth:', error);
        });
        return;
      }
      setView({ providerId, session, status: 'waiting_for_user', error: null });
      for (const event of run.earlyEvents) {
        handleStatus(run, event);
      }
      run.earlyEvents = [];
    } catch (error) {
      if (run.cancelled || runRef.current !== run) return;
      run.terminal = true;
      run.unlisten?.();
      setView({
        providerId,
        session: run.session,
        status: 'error',
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }, [close, handleStatus]);

  return { view, start, close };
}
