// React 19 dropped the global JSX namespace. Components return
// `ReactElement` or `ReactNode` directly instead.
import type { ReactElement } from 'react';

export type Component = ReactElement;