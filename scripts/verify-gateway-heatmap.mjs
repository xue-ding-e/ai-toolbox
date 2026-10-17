import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyGatewayHeatmap } from '../web/test/features/coding/gateway/gatewayHeatmapBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL('../web/test/features/coding/gateway/fixtures', import.meta.url));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'GatewayHeatmapFixture.jsx',
  artifactPrefix: 'gateway-heatmap-',
  verify: verifyGatewayHeatmap,
  fixtureAliases: [],
});
