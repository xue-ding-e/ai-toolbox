import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyConnectivityModalLayout } from '../web/test/features/coding/opencode/components/connectivityModalLayoutBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL('../web/test/features/coding/opencode/components/fixtures', import.meta.url));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ConnectivityTestModalFixture.jsx',
  artifactPrefix: 'connectivity-modal-layout-',
  verify: verifyConnectivityModalLayout,
});
