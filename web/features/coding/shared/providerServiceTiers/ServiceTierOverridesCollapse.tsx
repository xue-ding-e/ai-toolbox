import React from 'react';
import { Plus, Trash2, Zap } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import ProviderConfigCollapse from '@/features/coding/shared/providerConfig/ProviderConfigCollapse';
import {
  emptyServiceTierOverrideEntry,
  SERVICE_TIER_OVERRIDE_OPTIONS,
  type ModelServiceTierEntry,
  type ServiceTierOverridesState,
} from './serviceTierOverridesUtils';
import styles from './ServiceTierOverridesCollapse.module.less';

interface ServiceTierOverridesCollapseProps {
  value: ServiceTierOverridesState;
  onChange: (value: ServiceTierOverridesState) => void;
  className?: string;
}

/**
 * Provider-level per-model service (speed) tier override editor (issue #414).
 * Mirrors the `ModelRewritesCollapse` pattern: a standalone collapse (collapsed
 * by default) with a "use custom" toggle in the actions slot; the body holds a
 * list of `model -> tier` rows.
 *
 * Unlike the model catalog's speed-tier declaration (which only tells Codex
 * which tiers the model advertises), this forces the tier at the gateway: when
 * the upstream model matches and the request did not already ask for a premium
 * tier, the gateway writes `service_tier` before forwarding. Only OpenAI
 * Chat / Responses targets honour it.
 */
const ServiceTierOverridesCollapse: React.FC<ServiceTierOverridesCollapseProps> = ({
  value,
  onChange,
  className,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = React.useState(false);

  React.useEffect(() => {
    if (value.enabled) {
      setExpanded(true);
    }
  }, [value.enabled]);

  const update = React.useCallback(
    (patch: Partial<ServiceTierOverridesState>) => {
      onChange({ ...value, ...patch });
    },
    [onChange, value],
  );

  const updateRow = (index: number, patch: Partial<ModelServiceTierEntry>) => {
    const next = value.overrides.map((row, i) =>
      i === index ? { ...row, ...patch } : row,
    );
    update({ overrides: next });
  };

  const addRow = () => {
    update({ overrides: [...value.overrides, emptyServiceTierOverrideEntry()] });
  };

  const removeRow = (index: number) => {
    const next = value.overrides.filter((_, i) => i !== index);
    update({ overrides: next.length > 0 ? next : [emptyServiceTierOverrideEntry()] });
  };

  const disabled = !value.enabled;

  return (
    <ProviderConfigCollapse
      className={className}
      title={t('providerServiceTiers.title')}
      expanded={expanded}
      onExpandedChange={setExpanded}
      icon={<Zap />}
      actions={
        <div
          className={styles.toggleWrap}
          onClick={(event) => event.stopPropagation()}
        >
          <span>{t('providerServiceTiers.useCustom')}</span>
          <button
            type="button"
            className={`${styles.toggleButton} ${value.enabled ? styles.toggleButtonActive : ''}`}
            role="switch"
            aria-checked={value.enabled}
            onClick={() => {
              const enabled = !value.enabled;
              update({ enabled });
              if (enabled) {
                setExpanded(true);
              }
            }}
          >
            <span className={styles.toggleKnob} />
          </button>
        </div>
      }
    >
      <p className={styles.description}>{t('providerServiceTiers.description')}</p>
      <div className={styles.rows}>
        {value.overrides.map((row, index) => (
          <div key={index} className={styles.row}>
            <input
              className={styles.fieldInput}
              type="text"
              autoComplete="off"
              placeholder={t('providerServiceTiers.modelPlaceholder')}
              value={row.model}
              disabled={disabled}
              onChange={(event) => updateRow(index, { model: event.target.value })}
            />
            <span className={styles.arrow}>→</span>
            <select
              className={styles.tierSelect}
              value={row.serviceTier}
              disabled={disabled}
              onChange={(event) => updateRow(index, { serviceTier: event.target.value })}
            >
              {SERVICE_TIER_OVERRIDE_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <button
              type="button"
              className={styles.iconButton}
              onClick={() => removeRow(index)}
              disabled={disabled}
              aria-label={t('providerServiceTiers.remove')}
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
      </div>
      <div className={styles.toolbar}>
        <button
          type="button"
          className={styles.addButton}
          onClick={addRow}
          disabled={disabled}
        >
          <Plus size={14} style={{ marginRight: 4, verticalAlign: -2 }} />
          {t('providerServiceTiers.add')}
        </button>
      </div>
    </ProviderConfigCollapse>
  );
};

export default ServiceTierOverridesCollapse;