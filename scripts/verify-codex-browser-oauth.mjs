import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyCodexBrowserOauth } from '../web/test/features/coding/codex/codexBrowserOauthBrowserChecks.mjs';

await runBrowserFixture({
  fixtureDirectory: fileURLToPath(new URL('../web/test/features/coding/codex/fixtures', import.meta.url)),
  fixtureFilename: 'CodexCardViewFixture.jsx',
  artifactPrefix: 'codex-browser-oauth-',
  verify: verifyCodexBrowserOauth,
});
