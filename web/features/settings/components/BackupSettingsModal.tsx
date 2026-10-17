import React from 'react';
import {
  Modal,
  Form,
  Input,
  Space,
  Button,
  InputNumber,
  Switch,
  App,
  List,
  Tag,
  Tooltip,
  Select,
  Popconfirm,
  Typography,
} from 'antd';
import {
  DeleteOutlined,
  EditOutlined,
  FileOutlined,
  FolderOutlined,
  FolderOpenOutlined,
  PlusOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettingsStore, type BackupType, type WebDAVConfigFE } from '@/stores';
import {
  listBackupFileFilterPathOptions,
  normalizeBackupCustomEntryPath,
  testWebDAVConnection,
  testRepositoryConnection,
  getBackupRepositorySettings,
  getBackupEncryptionStatus,
  type BackupCustomEntry,
  type BackupCustomEntryType,
  type BackupFileFilterPathOption,
  type BackupFileFilterRule,
  type BackupRepositoryConfigFE,
} from '@/services';
import { ManagementSegmented } from '@/features/coding/shared/management';
import {
  findRustRegexIncompatibility,
  type RustRegexIssue,
} from '../utils/rustRegexCompat';
import ScrollFadeHint from './ScrollFadeHint';
import styles from './BackupSettingsModal.module.less';

interface BackupSettingsModalProps {
  open: boolean;
  onClose: () => void;
}

interface BackupCustomEntryFormValues {
  name: string;
  entryType: BackupCustomEntryType;
  sourcePath: string;
  restorePath?: string;
  excludePatterns?: string[];
}

interface FileFilterRuleFormValues {
  tool: string;
  filePath: string;
}

interface RepositoryFormValues {
  platform: 'github' | 'gitee';
  owner: string;
  repository: string;
  branch: string;
  directory: string;
}

const TOOL_ORDER = ['opencode', 'claude', 'claude_desktop', 'codex', 'grok', 'geminicli', 'antigravity', 'zcode', 'kimi', 'openclaw', 'pi', 'oh_my_pi', 'omo_native', 'hermes', 'dsh'];

// Static keys (not template literals) so the i18n key scanner sees every one.
const EXCLUDE_PATTERN_ISSUE_MESSAGE_KEYS: Record<RustRegexIssue, string> = {
  lookAround: 'settings.backupSettings.customEntries.excludePatternUnsupported.lookAround',
  backreference: 'settings.backupSettings.customEntries.excludePatternUnsupported.backreference',
  namedBackreference:
    'settings.backupSettings.customEntries.excludePatternUnsupported.namedBackreference',
  subroutineCall:
    'settings.backupSettings.customEntries.excludePatternUnsupported.subroutineCall',
  literalQuote: 'settings.backupSettings.customEntries.excludePatternUnsupported.literalQuote',
  unsupportedEscape:
    'settings.backupSettings.customEntries.excludePatternUnsupported.unsupportedEscape',
};

const DEFAULT_REPOSITORY_FORM: RepositoryFormValues = {
  platform: 'github',
  owner: '',
  repository: '',
  branch: 'main',
  directory: 'ai-toolbox',
};

const BackupSettingsModal: React.FC<BackupSettingsModalProps> = ({
  open: isOpen,
  onClose,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [form] = Form.useForm();
  const [customEntryForm] = Form.useForm<BackupCustomEntryFormValues>();
  const {
    backupType,
    localBackupPath,
    webdav,
    backupEncryptionEnabled,
    backupImageAssetsEnabled,
    backupCliConfigFilesEnabled,
    backupCustomEntries,
    backupFileFilterRules,
    saveBackupSettingsUnified,
    autoBackupEnabled,
    autoBackupIntervalDays,
    autoBackupMaxKeep,
  } = useSettingsStore();

  const [currentBackupType, setCurrentBackupType] = React.useState<BackupType>(backupType);
  const [currentLocalPath, setCurrentLocalPath] = React.useState(localBackupPath);
  const [testingConnection, setTestingConnection] = React.useState(false);
  const [currentBackupImageAssetsEnabled, setCurrentBackupImageAssetsEnabled] =
    React.useState(backupImageAssetsEnabled);
  const [currentBackupCliConfigFilesEnabled, setCurrentBackupCliConfigFilesEnabled] =
    React.useState(backupCliConfigFilesEnabled);
  const [currentAutoBackupEnabled, setCurrentAutoBackupEnabled] = React.useState(autoBackupEnabled);
  const [currentIntervalDays, setCurrentIntervalDays] = React.useState(autoBackupIntervalDays);
  const [currentMaxKeep, setCurrentMaxKeep] = React.useState(autoBackupMaxKeep);
  const [currentBackupCustomEntries, setCurrentBackupCustomEntries] =
    React.useState<BackupCustomEntry[]>(backupCustomEntries);
  const [customEntryModalOpen, setCustomEntryModalOpen] = React.useState(false);
  const [editingCustomEntry, setEditingCustomEntry] = React.useState<BackupCustomEntry | null>(null);
  const [customEntryType, setCustomEntryType] = React.useState<BackupCustomEntryType>('file');

  // Backup encryption: one global switch shared by all storage channels. The
  // password itself lives in the OS credential store and is never echoed back;
  // an empty input keeps the stored credential.
  const [currentEncryptionEnabled, setCurrentEncryptionEnabled] = React.useState(
    backupEncryptionEnabled,
  );
  const [encryptionPassword, setEncryptionPassword] = React.useState('');
  /** Credential-store state on this machine. `unknown` means the OS credential
   * store could not be read — the UI must not claim "no password stored". */
  const [storedPasswordState, setStoredPasswordState] = React.useState<
    'yes' | 'no' | 'unknown'
  >('unknown');

  // Tokens are draft-only here; the backend owns the separate repository record
  // and only returns whether a credential was saved.
  const [repositoryToken, setRepositoryToken] = React.useState('');
  const [repositoryHasToken, setRepositoryHasToken] = React.useState(false);
  /** Stored connection for hidden fields. Null blocks repository writes until
   * loading succeeds, so an empty fallback cannot clear an existing connection. */
  const [loadedRepositoryConfig, setLoadedRepositoryConfig] =
    React.useState<BackupRepositoryConfigFE | null>(null);
  const [repositoryLoading, setRepositoryLoading] = React.useState(true);
  const repositoryPlatform = Form.useWatch(['repository', 'platform'], form);
  const hasTokenForPlatform = repositoryHasToken
    && repositoryPlatform === loadedRepositoryConfig?.platform;
  const [testingRepository, setTestingRepository] = React.useState(false);
  const [saving, setSaving] = React.useState(false);

  // Scroll fade hint: attach to the real modal body scroll container. A callback
  // ref is required — an isOpen effect alone fires before the antd portal has
  // inserted the dialog body on first open, so the ref would still be null there.
  const bodyRef = React.useRef<HTMLDivElement | null>(null);
  const [scrollContainer, setScrollContainer] = React.useState<HTMLElement | null>(null);
  const attachBody = React.useCallback((node: HTMLDivElement | null) => {
    bodyRef.current = node;
    setScrollContainer(node?.closest<HTMLElement>('.ant-modal-body') ?? null);
  }, []);
  React.useEffect(() => {
    if (!isOpen) {
      setScrollContainer(null);
      setEncryptionPassword('');
      setRepositoryToken('');
      setLoadedRepositoryConfig(null);
      setRepositoryLoading(true);
    }
  }, [isOpen]);

  // File filter rules state
  const [filterRuleForm] = Form.useForm<FileFilterRuleFormValues>();
  const [currentFileFilterRules, setCurrentFileFilterRules] =
    React.useState<BackupFileFilterRule[]>(backupFileFilterRules);
  const [filterRuleModalOpen, setFilterRuleModalOpen] = React.useState(false);
  const [filterPathOptions, setFilterPathOptions] = React.useState<BackupFileFilterPathOption[]>(
    []
  );
  const [filterPathOptionsLoading, setFilterPathOptionsLoading] = React.useState(false);
  const selectedFilterRuleTool = Form.useWatch('tool', filterRuleForm) as string | undefined;

  const loadFilterPathOptions = React.useCallback(async () => {
    setFilterPathOptionsLoading(true);
    try {
      setFilterPathOptions(await listBackupFileFilterPathOptions());
    } catch (error) {
      console.error('Failed to load backup file filter path options:', error);
      setFilterPathOptions([]);
    } finally {
      setFilterPathOptionsLoading(false);
    }
  }, []);

  const filterToolOptions = React.useMemo(() => {
    const tools = Array.from(new Set(filterPathOptions.map((option) => option.tool)));
    tools.sort((a, b) => {
      const indexA = TOOL_ORDER.indexOf(a);
      const indexB = TOOL_ORDER.indexOf(b);
      if (indexA === -1 && indexB === -1) return a.localeCompare(b);
      if (indexA === -1) return 1;
      if (indexB === -1) return -1;
      return indexA - indexB;
    });

    return tools.map((tool) => ({
      value: tool,
      label: tool,
    }));
  }, [filterPathOptions]);

  const selectedFilterPathOptions = React.useMemo(
    () => filterPathOptions.filter((option) => option.tool === selectedFilterRuleTool),
    [filterPathOptions, selectedFilterRuleTool]
  );

  const selectedToolExistingFilterPaths = React.useMemo(() => {
    if (!selectedFilterRuleTool) return new Set<string>();

    return new Set(
      currentFileFilterRules
        .filter((rule) => rule.tool === selectedFilterRuleTool)
        .map((rule) => rule.file_path)
    );
  }, [currentFileFilterRules, selectedFilterRuleTool]);

  React.useEffect(() => {
    let cancelled = false;
    if (isOpen) {
      setCurrentBackupType(backupType);
      setCurrentLocalPath(localBackupPath);
      setCurrentBackupImageAssetsEnabled(backupImageAssetsEnabled);
      setCurrentBackupCliConfigFilesEnabled(backupCliConfigFilesEnabled);
      setCurrentAutoBackupEnabled(autoBackupEnabled);
      setCurrentIntervalDays(autoBackupIntervalDays);
      setCurrentMaxKeep(autoBackupMaxKeep);
      setCurrentBackupCustomEntries(backupCustomEntries);
      setCurrentFileFilterRules(backupFileFilterRules);
      setCurrentEncryptionEnabled(backupEncryptionEnabled);
      setEncryptionPassword('');
      setRepositoryToken('');
      setRepositoryLoading(true);
      setLoadedRepositoryConfig(null);
      setStoredPasswordState('unknown');
      form.setFieldsValue({
        backupType,
        webdav,
        repository: DEFAULT_REPOSITORY_FORM,
      });
      void loadFilterPathOptions();

      // Load repository connection + credential-store status (token never returns).
      void (async () => {
        try {
          const repositoryView = await getBackupRepositorySettings();
          if (cancelled) return;
          setRepositoryHasToken(repositoryView.has_token);
          setLoadedRepositoryConfig(repositoryView.config);
          const stored = repositoryView.config;
          form.setFieldsValue({
            repository: {
              platform: stored.platform,
              owner: stored.owner,
              repository: stored.repository,
              branch: stored.branch || DEFAULT_REPOSITORY_FORM.branch,
              // Empty is a valid repository root, not a missing setting.
              directory: stored.owner || stored.repository
                ? stored.directory
                : stored.directory || DEFAULT_REPOSITORY_FORM.directory,
            } satisfies RepositoryFormValues,
          });
          setRepositoryToken('');
        } catch (error) {
          if (cancelled) return;
          console.error('Failed to load backup repository settings:', error);
          setRepositoryHasToken(false);
          message.error(t('settings.backupSettings.repository.errors.loadSettings'));
        } finally {
          if (!cancelled) setRepositoryLoading(false);
        }
      })();
      void (async () => {
        try {
          const encryptionStatus = await getBackupEncryptionStatus();
          if (cancelled) return;
          setStoredPasswordState(
            encryptionStatus.password_known
              ? (encryptionStatus.has_password ? 'yes' : 'no')
              : 'unknown',
          );
        } catch (error) {
          if (cancelled) return;
          console.error('Failed to load backup encryption status:', error);
          setStoredPasswordState('unknown');
        }
      })();
    }
    return () => { cancelled = true; };
  }, [
    isOpen,
    backupType,
    localBackupPath,
    webdav,
    backupEncryptionEnabled,
    backupImageAssetsEnabled,
    backupCliConfigFilesEnabled,
    backupCustomEntries,
    backupFileFilterRules,
    autoBackupEnabled,
    autoBackupIntervalDays,
    autoBackupMaxKeep,
    form,
    loadFilterPathOptions,
    message,
    t,
  ]);

  const handleSelectFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t('settings.backupSettings.selectFolder'),
      });
      if (selected) {
        setCurrentLocalPath(selected as string);
      }
    } catch {
      // User cancelled
    }
  };

  // Backend failures arrive as JSON `{message, suggestion}`; showing only the
  // translated suggestion used to drop the concrete detail (entry name,
  // pattern, URL) the message carries, so keep both halves.
  const describeBackendError = (error: unknown): string => {
    try {
      const errorObj = JSON.parse(String(error));
      const detail = typeof errorObj.message === 'string' ? errorObj.message : '';
      const suggestion = typeof errorObj.suggestion === 'string' ? t(errorObj.suggestion) : '';
      const parts = [detail, suggestion].filter(Boolean);
      return parts.length > 0 ? parts.join(' — ') : String(error);
    } catch {
      return String(error);
    }
  };

  const handleSave = async () => {
    if (saving || repositoryLoading
      || (currentBackupType === 'repository' && !loadedRepositoryConfig)) return;
    setSaving(true);
    try {
      await form.validateFields();
      // validateFields only returns fields that are currently mounted. The webdav
      // and repository form items render conditionally per storage channel, so the
      // other channel's values come from the preserved form store (populated on
      // open), falling back to the loaded store values — never to an undefined
      // shape that would wipe the saved connection.
      const allValues = form.getFieldsValue(true);
      const webdavValues = (allValues.webdav ?? webdav) as WebDAVConfigFE;
      const repositoryValues = (currentBackupType === 'repository'
        ? allValues.repository ?? loadedRepositoryConfig
        : loadedRepositoryConfig) ?? DEFAULT_REPOSITORY_FORM;
      await saveBackupSettingsUnified({
        backupType: currentBackupType,
        localBackupPath: currentLocalPath,
        webdav: webdavValues,
        backupEncryptionEnabled: currentEncryptionEnabled,
        encryptionPassword: encryptionPassword || undefined,
        repository: repositoryValues,
        repositoryToken: currentBackupType === 'repository'
          ? repositoryToken || undefined
          : undefined,
        backupImageAssetsEnabled: currentBackupImageAssetsEnabled,
        backupCliConfigFilesEnabled: currentBackupCliConfigFilesEnabled,
        backupCustomEntries: currentBackupCustomEntries,
        backupFileFilterRules: currentFileFilterRules,
        autoBackupEnabled: currentAutoBackupEnabled,
        autoBackupIntervalDays: currentIntervalDays,
        autoBackupMaxKeep: currentMaxKeep,
      });
      // Never keep the submitted password in frontend state.
      setEncryptionPassword('');
      onClose();
    } catch (error) {
      // Form validation failures already show inline messages; report backend
      // failures and keep the draft so the user can adjust instead of retyping.
      if (error && typeof error === 'object' && 'errorFields' in error) {
        return;
      }
      console.error('Failed to save backup settings:', error);
      message.error(`${t('settings.backupSettings.saveFailed')}: ${describeBackendError(error)}`);
    } finally {
      setSaving(false);
    }
  };

  const handleBackupTypeChange = (value: BackupType) => {
    setCurrentBackupType(value);
  };

  const handleTestRepositoryConnection = async () => {
    try {
      const values = await form.validateFields(['repository']);
      setTestingRepository(true);
      await testRepositoryConnection(
        values.repository as BackupRepositoryConfigFE,
        repositoryToken || undefined,
      );
      message.success(t('settings.webdav.testSuccess'));
    } catch (error) {
      if (error && typeof error === 'object' && 'errorFields' in error) {
        return;
      }
      console.error('Repository connection test failed:', error);
      message.error(`${t('settings.webdav.testFailed')}: ${describeBackendError(error)}`);
    } finally {
      setTestingRepository(false);
    }
  };

  const normalizeCustomEntryPath = async (path: string): Promise<string> => {
    const trimmedPath = path.trim();
    if (!trimmedPath) {
      return '';
    }

    try {
      return await normalizeBackupCustomEntryPath(trimmedPath);
    } catch (error) {
      console.error('Failed to normalize custom backup path:', error);
      return trimmedPath;
    }
  };

  const handleOpenCustomEntryModal = (entry?: BackupCustomEntry) => {
    const nextEntryType = entry?.entry_type ?? 'file';
    setEditingCustomEntry(entry ?? null);
    setCustomEntryType(nextEntryType);
    customEntryForm.setFieldsValue({
      name: entry?.name ?? '',
      entryType: nextEntryType,
      sourcePath: entry?.source_path ?? '',
      restorePath: entry?.restore_path ?? '',
      excludePatterns: entry?.exclude_patterns ?? [],
    });
    setCustomEntryModalOpen(true);
  };

  const handleSelectCustomEntrySource = async () => {
    try {
      const selected = await open({
        directory: customEntryType === 'directory',
        multiple: false,
        title: customEntryType === 'directory'
          ? t('settings.backupSettings.customEntries.selectDirectory')
          : t('settings.backupSettings.customEntries.selectFile'),
      });
      if (selected && typeof selected === 'string') {
        customEntryForm.setFieldsValue({
          sourcePath: await normalizeCustomEntryPath(selected),
        });
      }
    } catch {
      // User cancelled
    }
  };

  const handleSaveCustomEntry = async () => {
    try {
      const values = await customEntryForm.validateFields();
      const sourcePath = await normalizeCustomEntryPath(values.sourcePath);
      const restorePath = values.restorePath?.trim()
        ? await normalizeCustomEntryPath(values.restorePath)
        : null;
      // Exclusions only apply to directories; switching to a file drops them.
      const excludePatterns = values.entryType === 'directory'
        ? (values.excludePatterns ?? [])
          .map((pattern) => pattern.trim())
          .filter((pattern) => pattern.length > 0)
        : [];
      const nextEntry: BackupCustomEntry = {
        id: editingCustomEntry?.id ?? `custom-backup-${Date.now()}`,
        name: values.name.trim(),
        source_path: sourcePath,
        restore_path: restorePath,
        entry_type: values.entryType,
        enabled: editingCustomEntry?.enabled ?? true,
        exclude_patterns: excludePatterns,
      };

      setCurrentBackupCustomEntries((entries) => {
        if (!editingCustomEntry) {
          return [...entries, nextEntry];
        }
        return entries.map((entry) => entry.id === editingCustomEntry.id ? nextEntry : entry);
      });
      setCustomEntryModalOpen(false);
    } catch {
      // Validation failed
    }
  };

  const handleToggleCustomEntry = (entryId: string, enabled: boolean) => {
    setCurrentBackupCustomEntries((entries) => entries.map((entry) => (
      entry.id === entryId ? { ...entry, enabled } : entry
    )));
  };

  const handleDeleteCustomEntry = (entryId: string) => {
    setCurrentBackupCustomEntries((entries) => entries.filter((entry) => entry.id !== entryId));
  };

  // File filter rule handlers
  const handleDeleteFilterRule = (index: number) => {
    setCurrentFileFilterRules((rules) => rules.filter((_, i) => i !== index));
  };

  const handleOpenFilterRuleModal = () => {
    filterRuleForm.resetFields();
    void loadFilterPathOptions();
    setFilterRuleModalOpen(true);
  };

  const handleSaveFilterRule = async () => {
    try {
      const values = await filterRuleForm.validateFields();
      const newRule: BackupFileFilterRule = {
        tool: values.tool,
        file_path: values.filePath.trim(),
      };

      setCurrentFileFilterRules((rules) => (
        rules.some((rule) => rule.tool === newRule.tool && rule.file_path === newRule.file_path)
          ? rules
          : [...rules, newRule]
      ));
      setFilterRuleModalOpen(false);
    } catch {
      // Validation failed
    }
  };

  const handleTestConnection = async () => {
    try {
      const values = await form.validateFields(['webdav']);
      const webdavConfig = values.webdav as Partial<WebDAVConfigFE>;

      if (!webdavConfig.url) {
        message.warning(t('settings.webdav.errors.checkUrl'));
        return;
      }

      setTestingConnection(true);
      await testWebDAVConnection(
        webdavConfig.url,
        webdavConfig.username || '',
        webdavConfig.password || '',
        webdavConfig.remotePath || ''
      );
      message.success(t('settings.webdav.testSuccess'));
    } catch (error) {
      console.error('WebDAV connection test failed:', error);
      message.error(`${t('settings.webdav.testFailed')}: ${describeBackendError(error)}`);
    } finally {
      setTestingConnection(false);
    }
  };

  return (
    <>
    <Modal
      title={t('settings.backupSettings.title')}
      open={isOpen}
      onOk={handleSave}
      onCancel={onClose}
      width={680}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      confirmLoading={saving}
      okButtonProps={{
        disabled: repositoryLoading
          || (currentBackupType === 'repository' && !loadedRepositoryConfig),
      }}
      cancelButtonProps={{ disabled: saving }}
      destroyOnHidden
      afterOpenChange={(opened) => {
        // Safety re-bind once the dialog is fully mounted and animated in, in case
        // the portal wrapper was not addressable when the body ref attached.
        if (opened && bodyRef.current) {
          setScrollContainer(
            bodyRef.current.closest<HTMLElement>('.ant-modal-body') ?? null,
          );
        }
      }}
    >
      <Form
        form={form}
        layout="horizontal"
        size="small"
        labelCol={{ flex: '108px' }}
        wrapperCol={{ flex: '1' }}
        labelAlign="left"
        labelWrap
        colon
        className={styles.form}
        disabled={saving || repositoryLoading}
      >
        <div className={styles.body} ref={attachBody}>
          <section className={styles.sectionCard}>
            <div className={styles.sectionToolbar}>
              <h3 className={styles.sectionTitle} style={{ marginBottom: 0 }}>
                {t('settings.backupSettings.storageType')}
              </h3>
              <ManagementSegmented<BackupType>
                value={currentBackupType}
                onChange={handleBackupTypeChange}
                ariaLabel={t('settings.backupSettings.storageType')}
                options={[
                  { value: 'local', label: t('settings.backupSettings.local') },
                  { value: 'webdav', label: t('settings.backupSettings.webdav') },
                  { value: 'repository', label: t('settings.backupSettings.repositoryChannel') },
                ]}
              />
            </div>

            {currentBackupType === 'local' && (
              <Form.Item label={t('settings.backupSettings.localPath')} style={{ marginTop: 10, marginBottom: 0 }}>
                <Space.Compact style={{ width: '100%' }}>
                  <Input
                    value={currentLocalPath}
                    readOnly
                    placeholder={t('settings.backupSettings.selectFolder')}
                    style={{ flex: 1 }}
                  />
                  <Button icon={<FolderOpenOutlined />} onClick={handleSelectFolder}>
                    {t('common.browse')}
                  </Button>
                </Space.Compact>
              </Form.Item>
            )}

            {currentBackupType === 'webdav' && (
              <div style={{ marginTop: 10 }}>
                <Form.Item label={t('settings.webdav.url')} name={['webdav', 'url']}>
                  <Input placeholder="https://dav.example.com" />
                </Form.Item>
                <Form.Item label={t('settings.webdav.username')} name={['webdav', 'username']}>
                  <Input />
                </Form.Item>
                <Form.Item label={t('settings.webdav.password')} name={['webdav', 'password']}>
                  <Input.Password visibilityToggle />
                </Form.Item>
                <Form.Item label={t('settings.webdav.remotePath')} name={['webdav', 'remotePath']}>
                  <Input placeholder="/backup" />
                </Form.Item>
                <Form.Item label={t('settings.webdav.hostLabel')} name={['webdav', 'hostLabel']}>
                  <Input placeholder={t('settings.webdav.hostLabelPlaceholder')} />
                </Form.Item>
                <div className={styles.testConnectionRow}>
                  <Button
                    onClick={handleTestConnection}
                    loading={testingConnection}
                  >
                    {testingConnection ? t('settings.webdav.testing') : t('settings.webdav.testConnection')}
                  </Button>
                </div>
              </div>
            )}

            {currentBackupType === 'repository' && (
              <div style={{ marginTop: 10 }}>
                <Form.Item label={t('settings.backupSettings.repositoryForm.platform')} name={['repository', 'platform']}>
                  <Select
                    onChange={() => setRepositoryToken('')}
                    options={[
                      { value: 'github', label: 'GitHub' },
                      { value: 'gitee', label: 'Gitee' },
                    ]}
                  />
                </Form.Item>
                <Form.Item
                  label={t('settings.backupSettings.repositoryForm.owner')}
                  name={['repository', 'owner']}
                >
                  <Input placeholder="owner" />
                </Form.Item>
                <Form.Item
                  label={t('settings.backupSettings.repositoryForm.repository')}
                  name={['repository', 'repository']}
                >
                  <Input placeholder="repository" />
                </Form.Item>
                <Form.Item
                  label={t('settings.backupSettings.repositoryForm.branch')}
                  name={['repository', 'branch']}
                >
                  <Input placeholder="main" />
                </Form.Item>
                <Form.Item
                  label={t('settings.backupSettings.repositoryForm.directory')}
                  name={['repository', 'directory']}
                >
                  <Input placeholder={t('settings.backupSettings.repositoryForm.directoryPlaceholder')} />
                </Form.Item>
                <Form.Item
                  label={t('settings.backupSettings.repositoryForm.token')}
                  required={false}
                >
                  <Space.Compact style={{ width: '100%' }}>
                    <Input.Password
                      visibilityToggle
                      value={repositoryToken}
                      onChange={(event) => setRepositoryToken(event.target.value)}
                      placeholder={
                        hasTokenForPlatform
                          ? t('settings.backupSettings.repositoryForm.tokenStoredPlaceholder')
                          : t('settings.backupSettings.repositoryForm.tokenPlaceholder')
                      }
                      autoComplete="new-password"
                    />
                  </Space.Compact>
                  <Typography.Text className={styles.helperText}>
                    {hasTokenForPlatform
                      ? t('settings.backupSettings.repositoryForm.tokenStored')
                      : t('settings.backupSettings.repositoryForm.tokenHint')}
                  </Typography.Text>
                </Form.Item>
                <div className={styles.testConnectionRow}>
                  <Button
                    onClick={handleTestRepositoryConnection}
                    loading={testingRepository}
                    disabled={repositoryLoading || !loadedRepositoryConfig}
                  >
                    {testingRepository
                      ? t('settings.webdav.testing')
                      : t('settings.webdav.testConnection')}
                  </Button>
                </div>
              </div>
            )}
          </section>

          <section className={styles.sectionCard}>
            <div className={styles.switchBlock}>
              <div className={styles.switchRow}>
                <span className={styles.switchLabel} style={{ fontWeight: 600 }}>
                  {t('settings.backupSettings.encryption.title')}
                </span>
                <Switch
                  checked={currentEncryptionEnabled}
                  onChange={setCurrentEncryptionEnabled}
                  aria-label={t('settings.backupSettings.encryption.title')}
                />
              </div>
              <Typography.Text className={styles.helperText}>
                {t('settings.backupSettings.encryption.description')}
              </Typography.Text>
            </div>
            {currentEncryptionEnabled && (
              <div className={styles.encryptionFields}>
                <Form.Item label={t('settings.backupSettings.encryption.password')} style={{ marginBottom: 0 }}>
                  <Input.Password
                    visibilityToggle
                    value={encryptionPassword}
                    onChange={(event) => setEncryptionPassword(event.target.value)}
                    placeholder={t('settings.backupSettings.encryption.passwordPlaceholder')}
                    autoComplete="new-password"
                  />
                  <Typography.Text className={styles.helperText}>
                    {storedPasswordState === 'yes'
                      ? t('settings.backupSettings.encryption.passwordStored')
                      : storedPasswordState === 'no'
                        ? t('settings.backupSettings.encryption.passwordNotSet')
                        : t('settings.backupSettings.encryption.passwordStateUnknown')}
                  </Typography.Text>
                </Form.Item>
              </div>
            )}
          </section>

          <section className={styles.sectionCard}>
            <h3 className={styles.sectionTitle}>{t('settings.backupSettings.scopeSectionTitle')}</h3>

            <div className={styles.switchBlock}>
              <div className={styles.switchRow}>
                <span className={styles.switchLabel}>{t('settings.backupSettings.imageAssets')}</span>
                <Switch
                  checked={currentBackupImageAssetsEnabled}
                  onChange={setCurrentBackupImageAssetsEnabled}
                  aria-label={t('settings.backupSettings.imageAssets')}
                />
              </div>
            </div>

            <div className={styles.switchBlock}>
              <div className={styles.switchRow}>
                <span className={styles.switchLabel}>{t('settings.backupSettings.cliConfigFiles')}</span>
                <Switch
                  checked={currentBackupCliConfigFilesEnabled}
                  onChange={setCurrentBackupCliConfigFilesEnabled}
                  aria-label={t('settings.backupSettings.cliConfigFiles')}
                />
              </div>
              <Typography.Text className={styles.helperText}>
                {t('settings.backupSettings.cliConfigFilesDesc')}
              </Typography.Text>
            </div>

            <div className={styles.subBlock}>
              <div className={styles.sectionToolbar}>
                <div className={styles.sectionMeta}>
                  <h4 className={styles.subTitle}>
                    {t('settings.backupSettings.customEntries.title')}
                  </h4>
                  <Typography.Text className={styles.helperText}>
                    {t('settings.backupSettings.customEntries.description')}
                  </Typography.Text>
                  {currentBackupCustomEntries.length === 0 && (
                    <span className={styles.listEmpty}>
                      {t('settings.backupSettings.customEntries.empty')}
                    </span>
                  )}
                </div>
                <Button
                  size="small"
                  icon={<PlusOutlined />}
                  onClick={() => handleOpenCustomEntryModal()}
                >
                  {t('settings.backupSettings.customEntries.add')}
                </Button>
              </div>
              {currentBackupCustomEntries.length > 0 && (
                <div className={styles.list}>
                  <List
                    size="small"
                    bordered
                    dataSource={currentBackupCustomEntries}
                    renderItem={(entry) => (
                      <List.Item
                        actions={[
                          <Switch
                            key="enabled"
                            size="small"
                            checked={entry.enabled}
                            onChange={(checked) => handleToggleCustomEntry(entry.id, checked)}
                          />,
                          <Tooltip
                            key="edit"
                            title={t('settings.backupSettings.customEntries.edit')}
                          >
                            <Button
                              type="text"
                              size="small"
                              icon={<EditOutlined />}
                              aria-label={t('settings.backupSettings.customEntries.edit')}
                              onClick={() => handleOpenCustomEntryModal(entry)}
                            />
                          </Tooltip>,
                          <Popconfirm
                            key="delete"
                            title={t('settings.backupSettings.customEntries.deleteConfirm')}
                            onConfirm={() => handleDeleteCustomEntry(entry.id)}
                            okText={t('common.delete')}
                            cancelText={t('common.cancel')}
                          >
                            <Tooltip title={t('settings.backupSettings.customEntries.delete')}>
                              <Button
                                danger
                                type="text"
                                size="small"
                                icon={<DeleteOutlined />}
                                aria-label={t('settings.backupSettings.customEntries.delete')}
                              />
                            </Tooltip>
                          </Popconfirm>,
                        ]}
                      >
                        <List.Item.Meta
                          avatar={entry.entry_type === 'directory' ? <FolderOutlined /> : <FileOutlined />}
                          title={(
                            <Space size="small" wrap>
                              <span>{entry.name}</span>
                              <Tag>
                                {entry.entry_type === 'directory'
                                  ? t('settings.backupSettings.customEntries.directory')
                                  : t('settings.backupSettings.customEntries.file')}
                              </Tag>
                            </Space>
                          )}
                          description={(
                            <Space direction="vertical" size={0} style={{ width: '100%' }}>
                              <Typography.Text
                                className={styles.pathLine}
                                ellipsis={{ tooltip: entry.source_path }}
                              >
                                {t('settings.backupSettings.customEntries.sourcePathShort')}: {entry.source_path}
                              </Typography.Text>
                              {entry.restore_path && (
                                <Typography.Text
                                  className={styles.pathLine}
                                  ellipsis={{ tooltip: entry.restore_path }}
                                >
                                  {t('settings.backupSettings.customEntries.restorePathShort')}: {entry.restore_path}
                                </Typography.Text>
                              )}
                              {entry.entry_type === 'directory' && entry.exclude_patterns.length > 0 && (
                                <Typography.Text
                                  className={styles.pathLine}
                                  ellipsis={{ tooltip: entry.exclude_patterns.join(', ') }}
                                >
                                  {t('settings.backupSettings.customEntries.excludePatternsShort')}: {entry.exclude_patterns.join(', ')}
                                </Typography.Text>
                              )}
                            </Space>
                          )}
                        />
                      </List.Item>
                    )}
                  />
                </div>
              )}
            </div>

            <div className={styles.subBlock}>
              <div className={styles.sectionToolbar}>
                <div className={styles.sectionMeta}>
                  <h4 className={styles.subTitle}>
                    {t('settings.backupSettings.fileFilterRules.title')}
                  </h4>
                  <Typography.Text className={styles.helperText}>
                    {t('settings.backupSettings.fileFilterRules.description')}
                  </Typography.Text>
                  {currentFileFilterRules.length === 0 && (
                    <span className={styles.listEmpty}>
                      {t('settings.backupSettings.fileFilterRules.empty')}
                    </span>
                  )}
                  {!currentBackupCliConfigFilesEnabled && (
                    <Typography.Text className={styles.helperText}>
                      {t('settings.backupSettings.fileFilterRules.disabledByCliConfigFiles')}
                    </Typography.Text>
                  )}
                </div>
                <Button
                  size="small"
                  icon={<PlusOutlined />}
                  onClick={() => handleOpenFilterRuleModal()}
                >
                  {t('settings.backupSettings.fileFilterRules.add')}
                </Button>
              </div>
              {currentFileFilterRules.length > 0 && (
                <div className={styles.list}>
                  <List
                    size="small"
                    bordered
                    dataSource={currentFileFilterRules}
                    renderItem={(rule, index) => (
                      <List.Item
                        actions={[
                          <Popconfirm
                            key="delete"
                            title={t('settings.backupSettings.fileFilterRules.deleteConfirm')}
                            onConfirm={() => handleDeleteFilterRule(index)}
                            okText={t('common.delete')}
                            cancelText={t('common.cancel')}
                          >
                            <Tooltip title={t('settings.backupSettings.fileFilterRules.delete')}>
                              <Button
                                danger
                                type="text"
                                size="small"
                                icon={<DeleteOutlined />}
                                aria-label={t('settings.backupSettings.fileFilterRules.delete')}
                              />
                            </Tooltip>
                          </Popconfirm>,
                        ]}
                      >
                        <List.Item.Meta
                          avatar={<FileOutlined />}
                          title={
                            <Space size="small" wrap>
                              <span>{rule.tool}</span>
                              <Tag>{rule.file_path}</Tag>
                            </Space>
                          }
                        />
                      </List.Item>
                    )}
                  />
                </div>
              )}
            </div>
          </section>

          <section className={styles.sectionCard}>
            <div className={styles.switchRow}>
              <span className={styles.switchLabel} style={{ fontWeight: 600 }}>
                {t('settings.autoBackup.title')}
              </span>
              <Switch
                checked={currentAutoBackupEnabled}
                onChange={setCurrentAutoBackupEnabled}
                aria-label={t('settings.autoBackup.title')}
              />
            </div>
            {currentAutoBackupEnabled && (
              <div className={styles.autoBackupFields}>
                <Form.Item label={t('settings.autoBackup.interval')} style={{ marginBottom: 0 }}>
                  <InputNumber
                    value={currentIntervalDays}
                    onChange={(v) => setCurrentIntervalDays(v && v >= 1 ? Math.floor(v) : 1)}
                    min={1}
                    precision={0}
                    style={{ width: 120 }}
                    addonAfter={t('settings.autoBackup.days')}
                  />
                </Form.Item>
                <Form.Item label={t('settings.autoBackup.maxKeep')} style={{ marginBottom: 0 }}>
                  <InputNumber
                    value={currentMaxKeep}
                    onChange={(v) => setCurrentMaxKeep(v != null && v >= 0 ? Math.floor(v) : 0)}
                    min={0}
                    precision={0}
                    style={{ width: 120 }}
                    addonAfter={t('settings.autoBackup.count')}
                  />
                  {currentMaxKeep === 0 && (
                    <Typography.Text className={styles.warningHint}>
                      {t('settings.autoBackup.unlimitedHint')}
                    </Typography.Text>
                  )}
                </Form.Item>
              </div>
            )}
          </section>
        </div>
        <ScrollFadeHint scrollContainer={scrollContainer} />
      </Form>
    </Modal>
    <Modal
      title={editingCustomEntry
        ? t('settings.backupSettings.customEntries.edit')
        : t('settings.backupSettings.customEntries.add')}
      open={customEntryModalOpen}
      onOk={handleSaveCustomEntry}
      onCancel={() => setCustomEntryModalOpen(false)}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      width={640}
      destroyOnHidden
    >
      <Form
        form={customEntryForm}
        layout="horizontal"
        size="small"
        labelCol={{ flex: '108px' }}
        wrapperCol={{ flex: '1' }}
        labelAlign="left"
        className={styles.nestedForm}
      >
        <Form.Item
          label={t('settings.backupSettings.customEntries.name')}
          name="name"
          rules={[{ required: true, message: t('settings.backupSettings.customEntries.nameRequired') }]}
        >
          <Input placeholder={t('settings.backupSettings.customEntries.namePlaceholder')} />
        </Form.Item>

        <Form.Item
          label={t('settings.backupSettings.customEntries.type')}
          name="entryType"
          rules={[{ required: true }]}
        >
          <Select
            onChange={(value: BackupCustomEntryType) => setCustomEntryType(value)}
            options={[
              {
                value: 'file',
                label: t('settings.backupSettings.customEntries.file'),
              },
              {
                value: 'directory',
                label: t('settings.backupSettings.customEntries.directory'),
              },
            ]}
          />
        </Form.Item>

        <Form.Item
          label={t('settings.backupSettings.customEntries.sourcePath')}
          name="sourcePath"
          rules={[{ required: true, message: t('settings.backupSettings.customEntries.sourcePathRequired') }]}
          extra={t('settings.backupSettings.customEntries.pathHint')}
        >
          <Input
            placeholder={t('settings.backupSettings.customEntries.sourcePathPlaceholder')}
            addonAfter={
              <Tooltip title={t('common.browse')}>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOpenOutlined />}
                  onClick={handleSelectCustomEntrySource}
                  style={{ margin: -7 }}
                />
              </Tooltip>
            }
          />
        </Form.Item>

        <Form.Item
          label={t('settings.backupSettings.customEntries.restorePath')}
          name="restorePath"
          extra={t('settings.backupSettings.customEntries.restorePathHint')}
        >
          <Input placeholder={t('settings.backupSettings.customEntries.restorePathPlaceholder')} />
        </Form.Item>

        {customEntryType === 'directory' && (
          <Form.Item
            label={t('settings.backupSettings.customEntries.excludePatterns')}
            extra={t('settings.backupSettings.customEntries.excludePatternsHint')}
          >
            <Form.List name="excludePatterns">
              {(fields, { add, remove }) => (
                <div className={styles.excludePatternsList}>
                  {fields.map((field) => (
                    <div key={field.key} className={styles.excludePatternsRow}>
                      <Form.Item
                        {...field}
                        rules={[
                          {
                            validator: (_, value: string) => {
                              const pattern = (value ?? '').trim();
                              if (!pattern) {
                                return Promise.resolve();
                              }
                              // Exclude patterns are compiled by the backend's
                              // Rust regex engine, not by `new RegExp`: the JS
                              // check used to accept look-around/backreferences
                              // (save then failed for the whole form) and to
                              // reject possessive quantifiers Rust supports.
                              const issue = findRustRegexIncompatibility(pattern);
                              if (issue) {
                                return Promise.reject(new Error(
                                  t(EXCLUDE_PATTERN_ISSUE_MESSAGE_KEYS[issue]),
                                ));
                              }
                              return Promise.resolve();
                            },
                          },
                        ]}
                      >
                        <Input
                          placeholder={t('settings.backupSettings.customEntries.excludePatternPlaceholder')}
                        />
                      </Form.Item>
                      <Button
                        type="text"
                        danger
                        size="small"
                        icon={<DeleteOutlined />}
                        onClick={() => remove(field.name)}
                        aria-label={t('settings.backupSettings.customEntries.deleteExcludePattern')}
                      />
                    </div>
                  ))}
                  <Button
                    type="dashed"
                    size="small"
                    block
                    icon={<PlusOutlined />}
                    onClick={() => add('')}
                  >
                    {t('settings.backupSettings.customEntries.addExcludePattern')}
                  </Button>
                </div>
              )}
            </Form.List>
          </Form.Item>
        )}
      </Form>
    </Modal>
    <Modal
      title={t('settings.backupSettings.fileFilterRules.addTitle')}
      open={filterRuleModalOpen}
      onOk={handleSaveFilterRule}
      onCancel={() => setFilterRuleModalOpen(false)}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      width={480}
      destroyOnHidden
    >
      <Form
        form={filterRuleForm}
        layout="horizontal"
        size="small"
        labelCol={{ flex: '108px' }}
        wrapperCol={{ flex: '1' }}
        labelAlign="left"
        className={styles.nestedForm}
      >
        <Form.Item
          label={t('settings.backupSettings.fileFilterRules.tool')}
          name="tool"
          rules={[
            { required: true, message: t('settings.backupSettings.fileFilterRules.toolRequired') },
          ]}
        >
          <Select
            loading={filterPathOptionsLoading}
            onChange={() => filterRuleForm.setFieldValue('filePath', undefined)}
            options={filterToolOptions}
          />
        </Form.Item>
        <Form.Item
          label={t('settings.backupSettings.fileFilterRules.filePath')}
          name="filePath"
          rules={[
            {
              required: true,
              message: t('settings.backupSettings.fileFilterRules.filePathRequired'),
            },
          ]}
        >
          <Select
            disabled={!selectedFilterRuleTool || filterPathOptionsLoading}
            loading={filterPathOptionsLoading}
            placeholder={
              selectedFilterRuleTool
                ? t('settings.backupSettings.fileFilterRules.filePathPlaceholder')
                : t('settings.backupSettings.fileFilterRules.selectToolFirst')
            }
            options={selectedFilterPathOptions.map((option) => ({
              value: option.file_path,
              label: option.file_path,
              disabled: selectedToolExistingFilterPaths.has(option.file_path),
            }))}
          />
        </Form.Item>
      </Form>
    </Modal>
    </>
  );
};

export default BackupSettingsModal;
