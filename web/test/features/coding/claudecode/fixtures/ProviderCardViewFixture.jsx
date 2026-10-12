/**
 * Mounts the **real** `ClaudeCodePage` against a stubbed Tauri IPC layer so the
 * provider list's card layout can be driven with real pointer events in a
 * browser.
 *
 * Why the real page and not a card fixture: the layout switch spans four files
 * — the page's `useProviderListViewPrefs` + `DndContext` strategy, the shared
 * `ProviderListSection` toolbar, the mapping layer's `viewMode` branch, and the
 * compact card's own sortable registration. A card-only fixture would have to
 * re-create the page half, which is exactly where the disagreements live. The
 * failures this guards ("the toggle renders but nothing changes", "the grid
 * card's buttons do not match the list card's") are invisible to any check that
 * does not mount the real page and click the real control.
 *
 * Only `ClaudeCodePage` is mounted, so the stub table below is the page's own
 * command set — an unstubbed command throws loudly instead of silently
 * returning `undefined`, which would turn a missing stub into a confusing
 * render failure.
 */
import './ProviderCardViewStubs.js';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { App, ConfigProvider, theme } from 'antd';
import i18n from '@/i18n';
import { useAppStore } from '@/stores/appStore';
import { updateGatewayProviderProfiles } from '@/features/coding/shared/gateway/providerProfiles';
// Imported before the page on purpose: the page sits in a real import cycle
// (`ClaudeCodePage` → `shared/providerShare` → … → `app/routes` →
// `routeConfig` → the page). The app entry (`web/App.tsx`) enters that cycle at
// `app/routes`, so the fixture must too; importing the page first instead
// throws `Cannot access 'ClaudeCodePage' before initialization`.
import '@/app/routes';
import ClaudeCodePage from '@/features/coding/claudecode/pages/ClaudeCodePage';
import gatewayProfiles from '../../../../../../tauri/resources/gateway_provider_profiles.json';
import '@/App.css';

const parameters = new URLSearchParams(location.search);
const language = parameters.get('language') || 'zh-CN';
const themeMode = parameters.get('theme') || 'light';
const resolvedTheme = themeMode === 'system'
  ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
  : themeMode;
/** Which provider record carries `isApplied` — the locate target and the card
 *  that must show the gateway/apply button instead of 应用. */
const appliedProviderId = parameters.get('applied') || 'provider-a';
/** Initial layout, so a check can start in either one without clicking first. */
const initialViewMode = parameters.get('viewMode') || 'list';
await i18n.changeLanguage(language);
useAppStore.setState({ language });
document.documentElement.dataset.theme = resolvedTheme;
updateGatewayProviderProfiles(gatewayProfiles);

/**
 * Providers covering the subtitle shapes the card view must handle:
 * Opus and Sonnet both configured (so the role preference is observable),
 * only Sonnet configured (so the default Opus role falls through to it), and
 * neither (so the card renders no subtitle line at all).
 */
const createProvider = (id, name, env, createdAt, meta) => ({
  id,
  name,
  category: 'custom',
  settingsConfig: JSON.stringify({ env }),
  extraSettingsConfig: '{}',
  isApplied: id === appliedProviderId,
  isDisabled: false,
  createdAt,
  updatedAt: createdAt,
  meta,
});

const state = {
  runId: parameters.get('runId'),
  requests: [],
  /** Origins the title link has asked the system browser to open, in order. */
  openedUrls: [],
  viewMode: initialViewMode,
  providers: [
    createProvider(
      'provider-a',
      'Provider Opus',
      {
        ANTHROPIC_BASE_URL: 'https://provider-a.example.test',
        // Long enough to overflow the sidebar's value column, so the checks can
        // see the middle-elision — and tell it apart from the copy button's
        // output, which must still be the whole key.
        ANTHROPIC_AUTH_TOKEN: 'sk-ant-fixture-0123456789abcdefghijklmnopqrstuvwxyz-TAIL',
        ANTHROPIC_DEFAULT_OPUS_MODEL: 'claude-opus-4-8',
        ANTHROPIC_DEFAULT_SONNET_MODEL: 'claude-sonnet-4-6',
      },
      '2026-01-04T00:00:00.000Z',
      // The protocol a real custom provider carries (`meta.apiFormat`, written
      // by the provider form). Only the detail sidebar states it, so this is
      // the record whose sidebar must show the label.
      { apiFormat: 'openai_chat' },
    ),
    createProvider(
      'provider-b',
      'Provider Sonnet Only',
      { ANTHROPIC_BASE_URL: 'https://provider-b.example.test', ANTHROPIC_AUTH_TOKEN: 'fixture-key-b', ANTHROPIC_DEFAULT_SONNET_MODEL: 'claude-sonnet-4-6' },
      '2026-01-03T00:00:00.000Z',
    ),
    createProvider(
      'provider-c',
      'Provider No Models',
      { ANTHROPIC_BASE_URL: 'https://provider-c.example.test', ANTHROPIC_AUTH_TOKEN: 'fixture-key-c' },
      '2026-01-02T00:00:00.000Z',
    ),
  ],
};

const gatewayCliStatus = {
  cli_key: 'claude',
  state: 'direct',
  dot: 'idle',
  can_takeover: true,
  can_restore_direct: false,
  gateway_origin: null,
  runtime_root: null,
  managed_targets: [],
  mode: null,
  primary_provider_id: null,
  provider_priorities: [],
  message: null,
};

const eventListeners = new Map();
let nextEventId = 0;

window.__TAURI_INTERNALS__ = {
  transformCallback(callback) {
    const id = nextEventId++;
    eventListeners.set(id, callback);
    return id;
  },
  unregisterCallback(id) {
    eventListeners.delete(id);
  },
  async invoke(command, args) {
    state.requests.push({ command, args: structuredClone(args ?? null) });
    switch (command) {
      // --- provider list ---------------------------------------------------
      case 'get_claude_config_path': return 'C:\\Users\\tester\\.claude\\settings.json';
      case 'get_claude_root_path_info': return { path: 'C:\\Users\\tester\\.claude', source: 'default' };
      case 'list_claude_providers': return structuredClone(state.providers);
      case 'reorder_claude_providers': {
        const byId = new Map(state.providers.map(provider => [provider.id, provider]));
        state.providers = args.ids.map(id => byId.get(id)).filter(Boolean);
        return null;
      }
      case 'apply_claude_config': return null;
      case 'toggle_claude_code_provider_disabled': return null;
      case 'launch_claude_provider_cli': return null;
      case 'read_claude_settings': return {};
      case 'get_claude_common_config': return null;

      // --- provider list UI state -----------------------------------------
      case 'get_provider_list_state':
        return {
          sort_modes: { claudecode: 'custom' },
          last_used: {},
          view_modes: { claudecode: state.viewMode },
        };
      case 'save_provider_sort_mode': return null;
      case 'save_provider_view_mode': {
        state.viewMode = args.mode;
        return null;
      }
      case 'record_provider_last_used': return null;
      case 'list_opencode_favorite_providers': return [];

      // --- gateway ---------------------------------------------------------
      case 'proxy_gateway_cli_status': return structuredClone(gatewayCliStatus);
      case 'proxy_gateway_cli_statuses': return [structuredClone(gatewayCliStatus)];
      case 'proxy_gateway_supported_cli_keys': return ['claude'];
      case 'proxy_gateway_status': return { running: false, mode: null };
      case 'proxy_gateway_get_settings': return {};

      // --- plugins / prompts / sessions ------------------------------------
      case 'get_claude_plugin_runtime_status':
        return { feature_enabled: false, runtime_available: false, runtime_error: null, plugin_count: 0 };
      case 'list_claude_installed_plugins': return [];
      case 'list_claude_known_marketplaces': return [];
      case 'get_claude_plugin_status': return { enabled: false };
      case 'get_claude_onboarding_status': return false;
      case 'list_claude_prompt_configs': return [];
      case 'list_tool_sessions':
        return { sessions: [], total: 0, has_more: false, next_cursor: null };

      // --- feature-availability probes -------------------------------------
      case 'has_all_api_hub_extension': return false;
      case 'has_cc_switch_db': return false;

      // --- app shell --------------------------------------------------------
      case 'refresh_tray_menu': return null;
      case 'get_settings': return null;
      case 'plugin:event|listen': return 1;
      case 'plugin:event|unlisten': return null;
      case 'plugin:opener|reveal_item_in_dir': return null;
      // The title link opens the provider's origin in the system browser. The
      // fixture records the call instead: `openUrl` is the only observable
      // effect, and a real opener would launch a browser window per click.
      case 'plugin:opener|open_url': {
        state.openedUrls.push(args.url);
        return null;
      }

      default:
        throw new Error('Unstubbed fixture command: ' + command);
    }
  },
};

const wait = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

/**
 * Clipboards are recorded rather than written to.
 *
 * The detail sidebar's copy buttons are only worth having if they put the
 * **right** text on the clipboard — a check that the button exists would pass
 * on a button wired to the masked value. A real clipboard needs a permission a
 * headless fixture would have to grant, so the writes are captured instead.
 */
const clipboardWrites = [];
Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: {
    writeText: async (text) => { clipboardWrites.push(text); },
  },
});

const cardFor = name => [...document.querySelectorAll('[data-provider-id]')]
  .find(card => card.textContent.includes(name));

/** The card's title element: the span whose text is exactly the provider name. */
const titleOf = (card, name) => [...(card?.querySelectorAll('span') ?? [])]
  .find(span => span.textContent.trim() === name);

window.providerCardViewFixture = {
  state,
  /**
   * The layout toggle's centre.
   *
   * Found by its `data-provider-view-toggle` attribute rather than by text:
   * the control is a single icon button (no label to match on), and its
   * accessible name is a sentence the copy can reword.
   */
  layoutToggleCenter() {
    const button = document.querySelector('[data-provider-view-toggle]');
    if (!button) return null;
    const rect = button.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  /** The layout currently in effect, as the toggle records it. */
  layoutToggleMode() {
    return document.querySelector('[data-provider-view-toggle]')
      ?.getAttribute('data-provider-view-toggle') ?? null;
  },
  /** The toggle's visible word — the layout a click switches to. */
  layoutToggleLabel() {
    return document.querySelector('[data-provider-view-toggle]')?.textContent.trim() ?? null;
  },
  /**
   * The icon the toggle currently shows, as antd's own icon name.
   *
   * antd stamps every icon's wrapper `<span class="anticon">` with
   * `aria-label="<icon name>"`, which is the honest handle here: the point of
   * the check is that the toggle no longer draws the 2×2-squares glyph that
   * `AppstoreOutlined` — the 通用配置 button — uses.
   */
  layoutToggleIconName() {
    return window.providerCardViewFixture.iconName(
      document.querySelector('[data-provider-view-toggle]'),
    );
  },
  /** The 通用配置 button's icon name, for comparison with the toggle's. */
  commonConfigIconName() {
    const button = [...document.querySelectorAll('button')]
      .find((element) => element.textContent.trim() === '通用配置');
    return window.providerCardViewFixture.iconName(button);
  },
  /** antd's icon name inside an element that carries one. */
  iconName(element) {
    return element?.querySelector('.anticon')?.getAttribute('aria-label') ?? null;
  },
  /** Provider names in the order the page renders them. */
  renderedOrder() {
    return [...document.querySelectorAll('[data-provider-id]')]
      .map(card => state.providers.find(provider => card.textContent.includes(provider.name))?.name)
      .filter(Boolean);
  },
  /** How many grid columns the card view laid out. */
  gridColumnCount() {
    const grid = document.querySelector('[data-provider-grid]');
    if (!grid) return 0;
    return getComputedStyle(grid).gridTemplateColumns.split(' ').filter(Boolean).length;
  },
  /** The card's own element (the box carrying the chrome), for geometry. */
  cardBox(name) {
    const card = cardFor(name);
    if (!card) return null;
    const rect = card.getBoundingClientRect();
    return { width: rect.width, height: rect.height, top: rect.top, left: rect.left };
  },
  /** The card's full visible text, for "does it still show X" checks. */
  cardText(name) {
    return cardFor(name)?.textContent ?? null;
  },
  /**
   * The second line of a card, or null when the card renders none.
   *
   * Every gap on the line is a CSS margin, not a text node, so `textContent`
   * would read `Opus:claude-opus-4-8Sonnet:claude-sonnet-4-6`. Rebuilding the
   * line from the parts reports what the user actually sees; asserting the
   * run-together form would pin these checks to the markup rather than to the
   * rendering.
   */
  subtitleText(name) {
    const card = cardFor(name);
    if (!card) return null;
    const subtitle = card.querySelector('[data-provider-subtitle]');
    if (!subtitle) return null;
    return [...subtitle.children]
      .map(entry => [...entry.children].map(part => part.textContent.trim()).join(' '))
      .join('  ');
  },
  /**
   * The mark the card draws between the second and each later model pair.
   *
   * Read from the pseudo-element rather than from text: the bar is punctuation
   * between two pairs, so it is not a child of the line — which also means
   * `subtitleText` below reports the pairs without it.
   */
  subtitleSeparator(name) {
    const card = cardFor(name);
    const entries = card?.querySelectorAll('[data-provider-subtitle] > *');
    const second = entries?.[1];
    return second ? getComputedStyle(second, '::before').content : null;
  },
  /** The state tags a card states beside its title, in order. */
  cardTagLabels(name) {
    const card = cardFor(name);
    const row = card?.querySelector('[data-provider-card-tags]');
    if (!row) return null;
    return [...row.querySelectorAll('.ant-tag')].map(tag => tag.textContent.trim());
  },
  /** Origins the title link has opened, in order. */
  openedUrls() {
    return [...state.openedUrls];
  },
  /** Visible footer action labels of a card, in render order. */
  footerActions(name) {
    const card = cardFor(name);
    if (!card) return null;
    const footer = card.querySelector('[data-provider-card-footer]');
    if (!footer) return null;
    return [...footer.querySelectorAll('button')]
      .map(button => button.textContent.trim())
      .filter(Boolean);
  },
  /**
   * The card's rendered type scale, as computed by the browser.
   *
   * The three zones have to sit at the Skills card's sizes — the card grid is
   * the same object as that one, and a title that renders a step larger than
   * the Skills card's (or a footer that does) reads as a different design.
   * Reading `getComputedStyle` rather than the stylesheet is the point: an
   * inline style anywhere in the chain would otherwise go unnoticed.
   */
  cardTypeScale(name) {
    const card = cardFor(name);
    if (!card) return null;
    const title = titleOf(card, name);
    const subtitle = card.querySelector('[data-provider-subtitle]');
    const button = card.querySelector('[data-provider-card-footer] .ant-btn');
    const tag = card.querySelector('[data-provider-card-tags] .ant-tag');
    return {
      title: title ? getComputedStyle(title).fontSize : null,
      titleWeight: title ? getComputedStyle(title).fontWeight : null,
      subtitle: subtitle ? getComputedStyle(subtitle).fontSize : null,
      footerButton: button ? getComputedStyle(button).fontSize : null,
      tag: tag ? getComputedStyle(tag).fontSize : null,
    };
  },
  /** The open detail sidebar, or null when none is open. */
  detailPanel() {
    return document.querySelector('[data-provider-detail-panel]');
  },
  /** The labels of the detail sidebar's footer buttons, in order. */
  detailFooterActions() {
    const footer = document.querySelector('[data-provider-detail-footer]');
    if (!footer) return null;
    return [...footer.querySelectorAll('button')]
      .map(button => button.textContent.trim())
      .filter(Boolean);
  },
  /** How large the sidebar's footer buttons render, in px. */
  detailFooterButtonFontSize() {
    const button = document.querySelector('[data-provider-detail-footer] .ant-btn');
    return button ? getComputedStyle(button).fontSize : null;
  },
  /**
   * The labels of the action row that shares the title's state line — the
   * card's own actions, minus the state tags that sit on the same line.
   */
  /**
   * The state tags the sidebar's title line states, in order.
   *
   * The same fragment the cell's tag row renders — a sidebar that listed the
   * tags by hand would drift the moment either side gained one.
   */
  detailStatusTagLabels() {
    const line = document.querySelector('[data-provider-detail-status]');
    if (!line) return null;
    return [...line.querySelectorAll('.ant-tag')].map((tag) => tag.textContent.trim());
  },
  detailStatusActions() {
    const line = document.querySelector('[data-provider-detail-status]');
    if (!line) return null;
    return [...line.querySelectorAll('button')]
      .map(button => button.textContent.trim())
      .filter(Boolean);
  },
  /** The detail sidebar's section titles, in order. */
  detailSectionTitles() {
    const panel = window.providerCardViewFixture.detailPanel();
    if (!panel) return null;
    return [...panel.querySelectorAll('[data-provider-detail-section-title]')]
      .map(title => title.textContent.trim());
  },
  /** Whether the sidebar's body shows the given text. */
  detailTextIncludes(text) {
    const panel = window.providerCardViewFixture.detailPanel();
    return panel ? panel.textContent.includes(text) : false;
  },
  /** Which connection fields the sidebar offers a copy button for, in order. */
  detailCopyFields() {
    const panel = window.providerCardViewFixture.detailPanel();
    if (!panel) return null;
    return [...panel.querySelectorAll('[data-provider-detail-copy]')]
      .map(button => button.getAttribute('data-provider-detail-copy'));
  },
  /** Clicks one of those copy buttons. Returns false when it is not there. */
  copyDetailField(field) {
    const panel = window.providerCardViewFixture.detailPanel();
    const button = panel?.querySelector(`[data-provider-detail-copy="${field}"]`);
    if (!button) return false;
    button.click();
    return true;
  },
  /** Every string the copy buttons have put on the clipboard, in order. */
  clipboardWrites() {
    return [...clipboardWrites];
  },
  /** The sidebar's rendered API-key line, as the user sees it. */
  detailKeyLine() {
    return window.providerCardViewFixture.detailFieldText('apiKey');
  },
  /** A field row's value, as rendered. */
  detailFieldText(field) {
    const panel = window.providerCardViewFixture.detailPanel();
    const cell = panel?.querySelector(`[data-provider-detail-field="${field}"]`);
    return cell?.textContent?.trim() ?? null;
  },
  closeDetail() {
    const panel = window.providerCardViewFixture.detailPanel();
    const button = panel?.querySelector('[data-provider-detail-close]');
    if (button) button.click();
  },
  /** The "more" trigger's opacity — 0 until the card is hovered. */
  moreButtonOpacity(name) {
    const card = cardFor(name);
    const wrapper = card?.querySelector('[data-provider-card-actions]');
    return wrapper ? Number(getComputedStyle(wrapper).opacity) : null;
  },
  moreButtonCenter(name) {
    const card = cardFor(name);
    const button = card?.querySelector('[data-provider-card-actions] button');
    if (!button) return null;
    const rect = button.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  cardCenter(name) {
    const card = cardFor(name);
    if (!card) throw new Error('No card rendered for: ' + name);
    const rect = card.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  /** The title's centre, for hovering and clicking the link itself. */
  titleCenter(name) {
    const title = titleOf(cardFor(name), name);
    if (!title) throw new Error('No title rendered for: ' + name);
    const rect = title.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  /** The tooltip antd rendered, or null while none is open. */
  openTooltipText() {
    return document.querySelector('.ant-tooltip:not(.ant-tooltip-hidden)')?.textContent?.trim() ?? null;
  },
  handleCenter(name) {
    const card = cardFor(name);
    const handle = card?.querySelector('[data-provider-card-handle]');
    if (!handle) throw new Error('No drag handle rendered for: ' + name);
    const rect = handle.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  },
  /** The menu items antd rendered for the open dropdown, in order. */
  openMenuItemLabels() {
    return [...document.querySelectorAll('.ant-dropdown-menu-item')]
      .map(item => item.textContent.trim());
  },
  lastViewModeSave() {
    const call = [...state.requests].reverse().find(entry => entry.command === 'save_provider_view_mode');
    return call ? call.args : null;
  },
  lastReorder() {
    const call = [...state.requests].reverse().find(entry => entry.command === 'reorder_claude_providers');
    return call ? call.args.ids : null;
  },
  storedOrder() {
    return state.providers.map(provider => provider.name);
  },
  async waitForOrder(names, timeoutMs = 4000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const order = window.providerCardViewFixture.renderedOrder();
      if (order.join('|') === names.join('|')) return order;
      await wait(50);
    }
    return window.providerCardViewFixture.renderedOrder();
  },
  async waitForSubtitle(name, expected, timeoutMs = 4000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const subtitle = window.providerCardViewFixture.subtitleText(name);
      if (subtitle === expected) return subtitle;
      await wait(50);
    }
    return window.providerCardViewFixture.subtitleText(name);
  },
};

createRoot(document.getElementById('root')).render(
  <ConfigProvider theme={{ algorithm: resolvedTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm }}>
    <App>
      {/* The cards call `useNavigate`, so the page needs router context. */}
      <MemoryRouter initialEntries={['/coding/claudecode']}>
        {/* App.css pins `html/body/#root` to `overflow: hidden` and the app
            scrolls `main`, so any check that drives the provider list needs
            this wrapper or it would assert against a page that cannot scroll. */}
        <main style={{ height: '100%', overflowY: 'auto' }}>
          <ClaudeCodePage />
        </main>
      </MemoryRouter>
    </App>
  </ConfigProvider>,
);
