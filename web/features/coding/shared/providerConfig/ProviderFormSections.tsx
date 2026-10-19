import React from 'react';
import { Form } from 'antd';
import { useTranslation } from 'react-i18next';
import BillingConfigCollapse from '@/features/coding/shared/providerBilling/BillingConfigCollapse';
import type { BillingConfigState } from '@/features/coding/shared/providerBilling/billingConfigUtils';
import CustomHeadersCollapse from '@/features/coding/shared/providerHeaders/CustomHeadersCollapse';
import type { CustomHeadersState } from '@/features/coding/shared/providerHeaders/customHeadersUtils';
import ModelRewritesCollapse from '@/features/coding/shared/providerModelRewrites/ModelRewritesCollapse';
import type { ModelRewritesState } from '@/features/coding/shared/providerModelRewrites/modelRewritesUtils';
import ServiceTierOverridesCollapse from '@/features/coding/shared/providerServiceTiers/ServiceTierOverridesCollapse';
import type { ServiceTierOverridesState } from '@/features/coding/shared/providerServiceTiers/serviceTierOverridesUtils';
import ProviderNotesCollapse from '@/features/coding/shared/providerConfig/ProviderNotesCollapse';

/**
 * Form-level wrapper column shared by every provider form section.
 * Sections are full-width rows; the label column belongs to the top fields.
 */
const PROVIDER_SECTION_WRAPPER_COL = { span: 24 };

export interface ProviderFormSectionsProps {
  /**
   * Whether the provider-specific sections are editable. Official-mode
   * providers keep only notes; everything below is hidden when false.
   */
  editable: boolean;

  /**
   * Advanced-settings block. Owned by the caller because the editor differs per
   * CLI (`JsonEditor` for Claude-style JSON settings, `TomlEditorFormItem` for
   * Codex `config.toml`), and it carries its own validation rules.
   */
  advancedSettings?: React.ReactNode;

  /**
   * Model-mapping block (Claude-style role → upstream model table). Codex
   * intentionally has none: its catalog lives on the provider card's model
   * list, so the mapping table is not rendered there. Callers that own one
   * pass it; the rest leave it undefined.
   */
  modelMapping?: React.ReactNode;

  /**
   * Billing section. Some flows (the import tab) deliberately omit it: the
   * imported provider has no pricing to edit yet. Defaults to following
   * `editable`.
   *
   * The three gateway-owned sections below share the same gate, and their value
   * props are only read when the matching `show*` is on — a caller that hides a
   * section may omit its state entirely.
   */
  showBilling?: boolean;
  billing?: BillingConfigState;
  onBillingChange?: (value: BillingConfigState) => void;
  /**
   * Also render the Fast / Ultrafast service-tier multipliers inside the billing
   * section. Only Codex passes this: its upstream is the OpenAI protocol, where
   * `service_tier` exists at all.
   */
  showBillingServiceTiers?: boolean;

  /**
   * Custom-header overrides. Like billing and rewrites, this is consumed only
   * by the local gateway — a CLI the gateway cannot take over has no reader for
   * it, so the section would collect values nothing acts on. Callers pass
   * `false` when the CLI is outside `GatewayCliKey::supported_mvp()`.
   */
  showCustomHeaders?: boolean;
  customHeaders?: CustomHeadersState;
  onCustomHeadersChange?: (value: CustomHeadersState) => void;

  /** Model rewrite rules. Gateway-only, same reasoning as `showCustomHeaders`. */
  showModelRewrites?: boolean;
  modelRewrites?: ModelRewritesState;
  onModelRewritesChange?: (value: ModelRewritesState) => void;

  /** Per-model speed tier overrides (issue #414). Gateway-only, same reasoning. */
  showModelServiceTiers?: boolean;
  modelServiceTiers?: ServiceTierOverridesState;
  onModelServiceTiersChange?: (value: ServiceTierOverridesState) => void;

  notesRows?: number;
  notesResetKey: string;
  /**
   * Whether the notes section renders at all. Defaults to `true`.
   *
   * OpenCode's provider record has no `notes` field, so rendering the editor
   * would collect text no save path stores — the "writes into a key nobody
   * reads" failure the SOP calls out. A CLI whose storage does have notes
   * leaves this unset.
   */
  showNotes?: boolean;
}

/**
 * Shared section layout for the provider edit modals.
 *
 * Locks the order every provider form uses after its top fields (category /
 * name / base URL / API key):
 *
 *   [model mapping]  ← Claude only, Codex keeps its catalog on the card
 *   [advanced settings]  ← caller-owned editor: JSON (Claude) vs TOML (Codex)
 *   [billing]
 *   [custom headers]
 *   [model rewrites]
 *   [notes]
 *
 * The top field block stays in each form: its fields, validation and
 * category-selection side effects differ enough per CLI that sharing it would
 * cost more than it saves.
 */
const ProviderFormSections: React.FC<ProviderFormSectionsProps> = ({
  editable,
  advancedSettings,
  modelMapping,
  showBilling = editable,
  billing,
  onBillingChange,
  showBillingServiceTiers = false,
  showCustomHeaders = editable,
  customHeaders,
  onCustomHeadersChange,
  showModelRewrites = editable,
  modelRewrites,
  onModelRewritesChange,
  showModelServiceTiers = editable,
  modelServiceTiers,
  onModelServiceTiersChange,
  notesRows = 3,
  notesResetKey,
  showNotes = true,
}) => {
  const { t } = useTranslation();

  return (
    <>
      {modelMapping}

      {editable && advancedSettings}

      {editable && showBilling && billing && onBillingChange && (
        <Form.Item wrapperCol={PROVIDER_SECTION_WRAPPER_COL}>
          <BillingConfigCollapse
            value={billing}
            onChange={onBillingChange}
            showServiceTierMultipliers={showBillingServiceTiers}
          />
        </Form.Item>
      )}

      {editable && showCustomHeaders && customHeaders && onCustomHeadersChange && (
        <Form.Item wrapperCol={PROVIDER_SECTION_WRAPPER_COL}>
          <CustomHeadersCollapse value={customHeaders} onChange={onCustomHeadersChange} />
        </Form.Item>
      )}

      {editable && showModelRewrites && modelRewrites && onModelRewritesChange && (
        <Form.Item wrapperCol={PROVIDER_SECTION_WRAPPER_COL}>
          <ModelRewritesCollapse value={modelRewrites} onChange={onModelRewritesChange} />
        </Form.Item>
      )}

      {editable
        && showModelServiceTiers
        && modelServiceTiers
        && onModelServiceTiersChange && (
        <Form.Item wrapperCol={PROVIDER_SECTION_WRAPPER_COL}>
          <ServiceTierOverridesCollapse
            value={modelServiceTiers}
            onChange={onModelServiceTiersChange}
          />
        </Form.Item>
      )}

      {showNotes && (
        <Form.Item name="notes" wrapperCol={PROVIDER_SECTION_WRAPPER_COL}>
          <ProviderNotesCollapse
            title={t('common.provider.notes')}
            placeholder={t('common.provider.notesPlaceholder')}
            rows={notesRows}
            resetKey={notesResetKey}
          />
        </Form.Item>
      )}
    </>
  );
};

export default ProviderFormSections;
