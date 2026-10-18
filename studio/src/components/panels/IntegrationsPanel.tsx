// Read-only integration list. Surfaces the exact status enum the backend
// exposes so the UI never invents capabilities it does not have.

import { useUiState } from '../../state/store';
import type { IntegrationStatus, IntegrationCategory } from '../../types/studio';
import type { ReactElement } from 'react';

const STATUS_LABEL: Record<IntegrationStatus, string> = {
  available_via_neutral_file: 'Neutral file',
  native_api_unavailable: 'Native API unavailable',
  not_configured: 'Not configured',
};

interface IntegrationsPanelProps {
  compact?: boolean;
  category?: IntegrationCategory;
}

export function IntegrationsPanel({ compact = false, category }: IntegrationsPanelProps): ReactElement {
  const all = useUiState((s) => s.server.integrations);
  const integrations = category ? all.filter((i) => i.category === category) : all;
  if (integrations.length === 0) {
    return <p className="panel-empty">Backend has not registered any integrations yet.</p>;
  }
  return (
    <ul className={compact ? 'panel-list panel-list-compact' : 'panel-list'}>
      {integrations.map((integration) => (
        <li key={integration.id} className="panel-list-item">
          <header>
            <span className="panel-list-name">{integration.name}</span>
            <span className={`panel-list-status panel-list-status-${integration.status}`}>
              {STATUS_LABEL[integration.status]}
            </span>
          </header>
          {compact ? null : (
            <p className="panel-list-meta">
              Category: {integration.category} · Platforms: {integration.platforms.join(', ') || 'unspecified'}
            </p>
          )}
          {integration.capabilities.length > 0 ? (
            <p className="panel-list-meta">Capabilities: {integration.capabilities.join(', ')}</p>
          ) : null}
          {integration.notes && !compact ? <p className="panel-list-notes">{integration.notes}</p> : null}
        </li>
      ))}
    </ul>
  );
}