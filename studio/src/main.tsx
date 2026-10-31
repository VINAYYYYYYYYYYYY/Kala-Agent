// Entry point. Mounts the React tree under a strict-mode root so we catch
// unsafe effects early in development. Production builds disable strict mode
// because double-mount during development is enough for our needs and we do
// not want extra renders in shipped code.
//
// Fonts are bundled locally through @fontsource so the CSP can stay at
// `font-src 'self' data:` with no remote Google Fonts request.

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/space-grotesk';
import '@fontsource-variable/newsreader';
import '@fontsource-variable/newsreader/wght-italic.css';
import '@fontsource-variable/jetbrains-mono';
import { App } from './App';
import { readStoredTheme } from './state/store';
import './styles/tokens.css';
import './styles/base.css';
import './styles/layout.css';
import './styles/studio.css';
import './styles/shell.css';
import './styles/composer.css';
import './styles/settings.css';

// Apply the persisted theme before the first paint so a Slate session does
// not flash the vellum sheet. Only the theme name is stored here.
document.documentElement.dataset.theme = readStoredTheme();

const root = document.getElementById('root');
if (!root) {
  throw new Error('studio: missing #root mount node');
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
