import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyZcodeCardView } from '../web/test/features/coding/zcode/zcodeCardViewBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/zcode/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ZcodeOfficialAccountFixture.jsx',
  artifactPrefix: 'zcode-card-view-',
  verify: verifyZcodeCardView,
  fixtureAliases: [],
});