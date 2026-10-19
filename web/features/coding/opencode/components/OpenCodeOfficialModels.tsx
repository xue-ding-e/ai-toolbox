import React from 'react';
import { Tag, Typography } from 'antd';
import { LockOutlined, SafetyOutlined } from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import type { OfficialModel } from '@/services/opencodeApi';

const { Text } = Typography;

interface OpenCodeOfficialModelsProps {
  /** `authProvidersData.mergedModels[providerId]`: official models for a provider that is also in the config. */
  models?: OfficialModel[];
  /** The detail sidebar draws the block without the list card's separating rules. */
  variant?: 'card' | 'detail';
}

const getStatusTagColor = (status: string): string => {
  switch (status) {
    case 'alpha':
      return 'purple';
    case 'beta':
      return 'blue';
    case 'deprecated':
      return 'red';
    default:
      return 'default';
  }
};

/**
 * The read-only official catalog merged into a channel.
 *
 * `auth.json` can cover a provider that the config also defines; those models
 * cannot be edited or deleted here, so they are kept out of the editable model
 * list and stated as their own block. The shared `ModelListSection` has no
 * read-only form, and the variant module's AGENTS.md names `footer` as the
 * place for exactly this (the same slot the Claude/Codex styles use).
 *
 * The previous inline copy hardcoded `#d4b106`; this one uses the warning
 * status token so it follows light/dark themes (DESIGN.md: no literal colors).
 */
const OpenCodeOfficialModels: React.FC<OpenCodeOfficialModelsProps> = ({
  models,
  variant = 'card',
}) => {
  const { t } = useTranslation();

  if (!models || models.length === 0) {
    return null;
  }

  const heading = (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 8 }}>
      <SafetyOutlined style={{ color: 'var(--color-status-warning)', fontSize: 12 }} />
      <Text type="secondary" style={{ fontSize: 11, color: 'var(--color-status-warning)' }}>
        {t('common.official.models')}
      </Text>
      <Text type="secondary" style={{ fontSize: 11 }}>
        ({models.length})
      </Text>
    </div>
  );

  return (
    <div
      data-official-models
      style={{
        marginTop: variant === 'card' ? 12 : 0,
        paddingTop: variant === 'card' ? 12 : 0,
        borderTop: variant === 'card' ? '1px solid var(--color-border-secondary)' : undefined,
      }}
    >
      {heading}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        {models.map((model) => {
          const limits = [
            model.context !== undefined && model.context !== null
              ? `${t('common.official.contextLimit')}: ${model.context.toLocaleString()}`
              : null,
            model.output !== undefined && model.output !== null
              ? `${t('common.official.outputLimit')}: ${model.output.toLocaleString()}`
              : null,
          ].filter(Boolean);

          return (
            <div
              key={model.id}
              title={t('common.official.modelReadOnlyHint')}
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
                padding: '6px 10px',
                border: '1px dashed var(--color-status-warning)',
                borderRadius: 4,
                background: 'var(--color-bg-container)',
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap', minWidth: 0 }}>
                <Text style={{ fontSize: 12 }}>{model.name || model.id}</Text>
                <Text type="secondary" style={{ fontSize: 11 }}>
                  ID: {model.id}
                </Text>
                {model.isFree && (
                  <Tag color="green" style={{ fontSize: 11, margin: 0 }}>
                    {t('common.official.freeModel')}
                  </Tag>
                )}
                {model.status && (
                  <Tag color={getStatusTagColor(model.status)} style={{ fontSize: 11, margin: 0 }}>
                    {model.status}
                  </Tag>
                )}
                {limits.length > 0 && (
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    {limits.join(' | ')}
                  </Text>
                )}
              </div>
              <LockOutlined style={{ fontSize: 12, color: 'var(--color-status-warning)', flexShrink: 0 }} />
            </div>
          );
        })}
      </div>
    </div>
  );
};

export default OpenCodeOfficialModels;
