import React from 'react';
import { message } from 'antd';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
  cancelCodexOfficialAccountOauth,
  startCodexOfficialAccountOauth,
  type CodexBrowserAuthStartResult,
} from '@/services/codexApi';

export type CodexBrowserOauthPhase =
  | 'starting' | 'waiting_for_user' | 'exchanging' | 'authorized' | 'cancelled' | 'expired' | 'error';

export interface CodexBrowserOauthView {
  providerId: string;
  phase: CodexBrowserOauthPhase;
  session: CodexBrowserAuthStartResult | null;
  error: string | null;
}

interface CodexAuthStatusEvent {
  sessionId: string;
  status: string;
  message?: string;
  accountId?: string;
}

interface BrowserOauthRun {
  providerId: string;
  session: CodexBrowserAuthStartResult | null;
  unlisten: UnlistenFn | null;
  earlyEvents: CodexAuthStatusEvent[];
  cancelled: boolean;
  terminal: boolean;
}

export function useCodexBrowserOauth(onCompleted: () => Promise<void>) {
  const [view, setView] = React.useState<CodexBrowserOauthView | null>(null);
  const runRef = React.useRef<BrowserOauthRun | null>(null);
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
      void cancelCodexOfficialAccountOauth(run.session.sessionId).catch((error) => {
        console.error('Failed to cancel Codex browser OAuth:', error);
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
      void cancelCodexOfficialAccountOauth(run.session.sessionId).catch(() => undefined);
    }
  }, []);

  const handleStatus = React.useCallback((run: BrowserOauthRun, event: CodexAuthStatusEvent) => {
    if (runRef.current !== run || run.cancelled || run.terminal) return;
    if (!run.session) {
      run.earlyEvents.push(event);
      return;
    }
    if (event.sessionId !== run.session.sessionId) return;
    if (event.status === 'waiting_for_user' || event.status === 'exchanging') {
      setView((current) => current?.providerId === run.providerId
        ? { ...current, phase: event.status as CodexBrowserOauthPhase, error: null } : current);
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
        console.error('Failed to reload Codex after browser OAuth:', error);
        message.error(error instanceof Error ? error.message : String(error));
      });
    } else if (event.status === 'cancelled') {
      runRef.current = null;
      setView(null);
    } else {
      setView((current) => current?.providerId === run.providerId
        ? { ...current, phase: event.status as CodexBrowserOauthPhase, error: event.message ?? null }
        : current);
    }
  }, []);

  const start = React.useCallback(async (providerId: string) => {
    close();
    const run: BrowserOauthRun = {
      providerId,
      session: null,
      unlisten: null,
      earlyEvents: [],
      cancelled: false,
      terminal: false,
    };
    runRef.current = run;
    setView({ providerId, phase: 'starting', session: null, error: null });
    try {
      // Install the listener before start: the callback may finish before IPC resolves.
      run.unlisten = await listen<CodexAuthStatusEvent>('codex-auth-status', (event) => {
        handleStatus(run, event.payload);
      });
      if (run.cancelled || runRef.current !== run) {
        run.unlisten();
        return;
      }
      const session = await startCodexOfficialAccountOauth(providerId);
      run.session = session;
      if (run.cancelled || runRef.current !== run) {
        void cancelCodexOfficialAccountOauth(session.sessionId).catch((error) => {
          console.error('Failed to cancel Codex browser OAuth:', error);
        });
        return;
      }
      setView({ providerId, phase: 'waiting_for_user', session, error: null });
      for (const event of run.earlyEvents) {
        handleStatus(run, event);
      }
      run.earlyEvents = [];
      if (!run.terminal) {
        await openUrl(session.authorizeUrl).catch((error) => {
          console.error('Failed to open Codex OAuth URL:', error);
          setView((current) => current?.session?.sessionId === session.sessionId
            ? { ...current, error: error instanceof Error ? error.message : String(error) }
            : current);
        });
      }
    } catch (error) {
      if (run.cancelled || runRef.current !== run) return;
      run.terminal = true;
      run.unlisten?.();
      setView({
        providerId,
        phase: 'error',
        session: run.session,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }, [close, handleStatus]);

  const reopenBrowser = React.useCallback(async () => {
    const url = runRef.current?.session?.authorizeUrl;
    if (!url) return;
    try {
      await openUrl(url);
      setView((current) => current ? { ...current, error: null } : current);
    } catch (error) {
      setView((current) => current
        ? { ...current, error: error instanceof Error ? error.message : String(error) }
        : current);
    }
  }, []);

  return { view, start, close, reopenBrowser };
}
