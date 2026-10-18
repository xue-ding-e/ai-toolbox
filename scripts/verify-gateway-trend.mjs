import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyGatewayTrendChart } from '../web/test/features/coding/gateway/gatewayTrendBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL('../web/test/features/coding/gateway/fixtures', import.meta.url));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'GatewayTrendFixture.jsx',
  artifactPrefix: 'gateway-trend-',
  verify: verifyGatewayTrendChart,
  fixtureAliases: [],
});
