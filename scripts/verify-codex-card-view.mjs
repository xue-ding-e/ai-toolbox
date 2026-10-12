import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyCodexCardView } from '../web/test/features/coding/codex/codexCardViewBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/codex/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'CodexCardViewFixture.jsx',
  artifactPrefix: 'codex-card-view-',
  verify: verifyCodexCardView,
  fixtureAliases: [],
});