import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyMigratedProviderCards } from '../web/test/features/coding/shared/providerCardVariants/migratedProviderCardsBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/shared/providerCardVariants/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'MigratedProviderCardsFixture.jsx',
  artifactPrefix: 'migrated-provider-cards-',
  verify: verifyMigratedProviderCards,
  fixtureAliases: [],
});
