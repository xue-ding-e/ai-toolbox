import React from 'react';
import { App, Button, Form, Modal, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import JsonEditor from '@/components/common/JsonEditor';

const { Text } = Typography;

export interface AuthConfigModalProps {
  open: boolean;
  /** `auth.json` 的原文（含明文密钥）。每次打开都从这里重读。 */
  content: string | undefined;
  /** 文件在磁盘上的位置，只用于在弹窗底部显示。 */
  path?: string;
  /**
   * 只读模式：凭据由别的存储管理时（OpenCode V2 的 `opencode.db`），
   * `auth.json` 只是查看入口，保存没有意义，也不该让用户以为改了会生效。
   * 只读时隐藏保存按钮与 `onSave` 路径。
   */
  readOnly?: boolean;
  /**
   * 落盘。第二个参数是**打开瞬间解析出的原始对象**：后端靠它做条目级合并
   * ——用户没动过的条目保留磁盘现值（OpenCode CLI 在编辑期间刷新的 token
   * 不会被旧副本覆盖），改过的写新值，删掉的才删。不做条目级合并的消费者
   * （如 OmO 的整份覆盖语义）忽略它即可。
   *
   * 各 CLI 的凭据语义不同（OmO 的值是 config value 语法、需要转义；OpenCode 的
   * 是字面量），所以转义与写入都在各自的后端命令里，这个组件不碰。
   */
  onSave: (
    config: Record<string, unknown>,
    original: Record<string, unknown>,
  ) => Promise<void>;
  onCancel: () => void;
  /** 保存成功后的刷新回调（重读配置 + 重查依赖凭据的列表）。 */
  onSaved: () => Promise<void> | void;
}

/**
 * 引擎凭据文件（`auth.json`）的应用内编辑弹窗。
 *
 * **为什么不做成「用资源管理器打开」**：本应用是配置管理器，密钥是它管理的对象
 * 之一，要能在应用内看到并修改（2026-10-07 用户明确要求）。跳系统文件管理器
 * 等于把编辑体验丢给记事本，而且用户在应用内根本看不到自己配了什么。
 *
 * 形态对齐各页面的「其他配置」区块（`JsonEditor` + 失焦/确定保存），只是放在
 * 弹窗里——凭据属于「偶尔改一次」的东西，不值得占页面版面。
 *
 * 目前有两个消费者：OpenCode 与 OmO Native 的「官方认证渠道」标题栏入口。
 */
const AuthConfigModal: React.FC<AuthConfigModalProps> = ({
  open,
  content,
  path,
  readOnly = false,
  onSave,
  onCancel,
  onSaved,
}) => {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [value, setValue] = React.useState<Record<string, unknown>>({});
  const [openedValue, setOpenedValue] = React.useState<Record<string, unknown>>({});
  // 磁盘原文解析失败时置位：此时 openedValue 不可信，且 value 为空的保存
  // 会整份清掉凭据文件，必须拦下（用户手输一份完整 JSON 重写仍被允许）。
  const [openedUnparseable, setOpenedUnparseable] = React.useState(false);
  const [valid, setValid] = React.useState(true);
  const [saving, setSaving] = React.useState(false);

  // 每次打开都从磁盘原文重读：这个弹窗不是常驻编辑面，拿着上次关闭时的副本
  // 会让用户在别处改过的内容被静默覆盖。
  React.useEffect(() => {
    if (!open) return;
    let parsed: Record<string, unknown> = {};
    let unparseable = false;
    if (content?.trim()) {
      try {
        const candidate = JSON.parse(content);
        if (candidate && typeof candidate === 'object' && !Array.isArray(candidate)) {
          parsed = candidate as Record<string, unknown>;
        } else {
          unparseable = true;
        }
      } catch (error) {
        console.error('Failed to parse auth.json:', error);
        unparseable = true;
      }
    }
    setValue(parsed);
    setOpenedValue(parsed);
    setOpenedUnparseable(unparseable);
    setValid(true);
  }, [open, content]);

  const handleSave = async () => {
    if (!valid) {
      message.error(t('common.jsonInvalid'));
      return;
    }
    // 磁盘原文坏到解析不出对象、编辑器又没被手填内容时，原样保存只会把凭据
    // 文件整份清空——宁可拦下让用户先修复 JSON。
    if (openedUnparseable && Object.keys(value).length === 0) {
      message.error(t('common.authConfig.unparseableBlocked'));
      return;
    }
    setSaving(true);
    try {
      await onSave(value, openedValue);
      message.success(t('common.success'));
      // 落盘成功后**立刻关弹窗**，刷新放后台跑。
      //
      // 刷新可能要重查依赖凭据的列表——OmO 的「引擎内建渠道」要逐个跑
      // `omo auth check`（引擎没有批量接口，48 个实测约 8 秒）。之前是
      // `await onSaved()` 之后才关，用户对着一个转圈的「保存」按钮等 8 秒，
      // 而数据其实早就写完了（2026-10-07 用户报告）。
      onCancel();
      void Promise.resolve(onSaved()).catch((error) => {
        // 刷新失败不该回滚已保存的内容，但要留下痕迹。
        console.error('Failed to refresh after saving auth.json:', error);
      });
    } catch (error) {
      console.error('Failed to save auth.json:', error);
      message.error(error instanceof Error ? error.message : t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      title={t(readOnly ? 'common.authConfig.viewTitle' : 'common.authConfig.title')}
      open={open}
      width={720}
      onCancel={onCancel}
      confirmLoading={saving}
      okText={t('common.save')}
      cancelText={t('common.cancel')}
      onOk={readOnly ? undefined : () => void handleSave()}
      footer={readOnly ? (
        <Button onClick={onCancel}>{t('common.close')}</Button>
      ) : undefined}
      destroyOnHidden
    >
      <Form layout="vertical">
        <Form.Item
          help={(
            <span style={{ fontSize: 12 }}>
              {t(readOnly ? 'common.authConfig.viewHint' : 'common.authConfig.hint')}
            </span>
          )}
          style={{ marginBottom: 0 }}
        >
          <JsonEditor
            value={value && Object.keys(value).length > 0 ? value : undefined}
            height={320}
            readOnly={readOnly}
            onChange={(next, isValid) => {
              setValue(
                next && typeof next === 'object' && !Array.isArray(next)
                  ? (next as Record<string, unknown>)
                  : {},
              );
              setValid(isValid);
            }}
          />
        </Form.Item>
        {path && (
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t('common.authConfig.path', { path })}
          </Text>
        )}
      </Form>
    </Modal>
  );
};

export default AuthConfigModal;
