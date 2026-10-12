import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyProviderCardView } from '../web/test/features/coding/claudecode/providerCardViewBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/claudecode/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ProviderCardViewFixture.jsx',
  artifactPrefix: 'provider-card-view-',
  verify: verifyProviderCardView,
  fixtureAliases: [],
});
