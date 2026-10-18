// Entry point. Mounts the React tree under a strict-mode root so we catch
// unsafe effects early in development. Production builds disable strict mode
// because double-mount during development is enough for our needs and we do
// not want extra renders in shipped code.

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './styles/studio.css';

const root = document.getElementById('root');
if (!root) {
  throw new Error('studio: missing #root mount node');
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);