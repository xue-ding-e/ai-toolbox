/**
 * Installs the Tauri host globals a mounted page expects.
 *
 * This module must be the fixture's **first** import: ES module bodies evaluate
 * in import order, and `app/routeConfig` reaches `MainLayout`, which reads
 * `platform()` at module scope. Assigning these globals in the fixture body
 * instead is too late — the layout module has already run and thrown.
 */
window.__TAURI_OS_PLUGIN_INTERNALS__ = { platform: 'windows' };
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener: () => {},
};
