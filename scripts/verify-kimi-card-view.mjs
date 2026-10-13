import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyKimiCardView } from '../web/test/features/coding/kimi/kimiCardViewBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/kimi/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'KimiOfficialAccountFixture.jsx',
  artifactPrefix: 'kimi-card-view-',
  verify: verifyKimiCardView,
  fixtureAliases: [],
});