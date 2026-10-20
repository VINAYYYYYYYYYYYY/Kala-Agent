// Vernier caliper slider — the tactile numeric control used by the Inspector.
//
// The rail is a real `role="slider"` with the full keyboard contract
// (arrows, PageUp/PageDown, Home/End). Ticks are derived from min/max/step;
// the triangular graphite thumb marks the value. Min, step and max are always
// printed, the value is a mono input, and the −/+ steppers change it by one
// step. Everything writes through `onChange`, so the Inspector still owns
// validation and the single store dispatch.

import { useRef } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent, ReactElement } from 'react';

export interface VernierTick {
  value: number;
  /** 0 at min, 1 at max. */
  ratio: number;
  major: boolean;
}

export interface VernierSliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit?: string | undefined;
  integer?: boolean | undefined;
  invalid?: boolean | undefined;
  onChange(next: number): void;
}

const MAX_TICKS = 33;

/**
 * Build a bounded tick rail from min/max/step. Every fifth tick is major.
 * The endpoints are always present. A huge range (0.1 … 2000 at step 0.5)
 * is sampled down to MAX_TICKS intervals instead of drawing 4000 marks.
 */
export function computeVernierTicks(min: number, max: number, step: number): VernierTick[] {
  if (![min, max, step].every(Number.isFinite) || max <= min) return [];
  const safeStep = step > 0 ? step : (max - min) / 10;
  const intervals = Math.floor((max - min) / safeStep + 1e-9);
  const stride = Math.max(1, Math.ceil((intervals + 1) / MAX_TICKS));
  const ticks: VernierTick[] = [];
  for (let i = 0; i <= intervals; i += stride) {
    const value = min + i * safeStep;
    ticks.push({
      value,
      ratio: (value - min) / (max - min),
      major: i % (stride * 5) === 0,
    });
  }
  const last = ticks[ticks.length - 1];
  if (!last || last.ratio < 1) {
    ticks.push({ value: max, ratio: 1, major: true });
  } else {
    last.major = true;
  }
  if (ticks.length > 0) ticks[0]!.major = true;
  return ticks;
}

/** Clamp to the range and snap to the step grid. */
export function snapToStep(value: number, min: number, max: number, step: number, integer = false): number {
  if (!Number.isFinite(value)) return min;
  const safeStep = step > 0 ? step : 0.1;
  let next = min + Math.round((value - min) / safeStep) * safeStep;
  if (integer) next = Math.round(next);
  next = Math.min(Math.max(next, min), max);
  // Kill binary floating-point dust from the step arithmetic.
  return Number(next.toFixed(6));
}

function formatNumber(value: number, step: number, integer?: boolean): string {
  if (!Number.isFinite(value)) return '—';
  if (integer) return String(Math.round(value));
  const decimals = step >= 1 ? 0 : Math.min(3, Math.max(1, String(step).split('.')[1]?.length ?? 1));
  return Number(value.toFixed(decimals)).toString();
}

export function VernierSlider(props: VernierSliderProps): ReactElement {
  const railRef = useRef<HTMLDivElement | null>(null);
  const { min, max, step } = props;
  const clamped = Math.min(Math.max(Number.isFinite(props.value) ? props.value : min, min), max);
  const ratio = max > min ? (clamped - min) / (max - min) : 0;
  const ticks = computeVernierTicks(min, max, step);
  const displayValue = Number.isFinite(props.value) ? props.value : '';
  const unitSuffix = props.unit ? ` ${props.unit}` : '';

  function commit(next: number): void {
    let value = Math.min(Math.max(next, min), max);
    if (props.integer) value = Math.round(value);
    else value = Number(value.toFixed(6));
    // Keep the current value's own phase; only pointer scrubbing snaps to the
    // min-anchored step grid, so ArrowRight on 40.0 (box width, step 0.5)
    // gives 40.5 rather than jumping to the 40.1 min-anchored mark.
    props.onChange(value);
  }

  function setFromPointer(event: ReactPointerEvent<HTMLDivElement>): void {
    const rect = railRef.current?.getBoundingClientRect();
    if (!rect || rect.width <= 0) return;
    const x = Math.min(Math.max(event.clientX - rect.left, 0), rect.width);
    props.onChange(snapToStep(min + (x / rect.width) * (max - min), min, max, step, props.integer));
  }

  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>): void {
    const big = step * 10;
    let next: number | null = null;
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowUp':
        next = clamped + step;
        break;
      case 'ArrowLeft':
      case 'ArrowDown':
        next = clamped - step;
        break;
      case 'PageUp':
        next = clamped + big;
        break;
      case 'PageDown':
        next = clamped - big;
        break;
      case 'Home':
        next = min;
        break;
      case 'End':
        next = max;
        break;
      default:
        return;
    }
    event.preventDefault();
    commit(next);
  }

  return (
    <div className="vernier" aria-label={`${props.label} vernier`}>
      <div className="vernier-head">
        <span className="vernier-label" title={`${props.label}${unitSuffix}`}>
          {props.label}{props.unit ? ` (${props.unit})` : ''}
        </span>
        <button
          type="button"
          className="vernier-stepper"
          aria-label={`Decrease ${props.label}`}
          onClick={() => commit(clamped - step)}
          disabled={clamped <= min}
        >
          −
        </button>
        <input
          className="vernier-input"
          type="number"
          inputMode={props.integer ? 'numeric' : 'decimal'}
          aria-label={`${props.label} value`}
          aria-invalid={props.invalid ? 'true' : undefined}
          step={step}
          min={min}
          max={max}
          value={displayValue}
          onChange={(event) => {
            const raw = event.target.value;
            if (raw === '') {
              props.onChange(0);
              return;
            }
            const numeric = Number(raw);
            if (Number.isFinite(numeric)) props.onChange(numeric);
          }}
        />
        <button
          type="button"
          className="vernier-stepper"
          aria-label={`Increase ${props.label}`}
          onClick={() => commit(clamped + step)}
          disabled={clamped >= max}
        >
          +
        </button>
      </div>

      <div
        ref={railRef}
        className="vernier-rail"
        role="slider"
        tabIndex={0}
        aria-label={`${props.label} vernier slider`}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={clamped}
        aria-valuetext={`${formatNumber(clamped, step, props.integer)}${unitSuffix}`}
        aria-invalid={props.invalid ? 'true' : undefined}
        aria-orientation="horizontal"
        onKeyDown={onKeyDown}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture?.(event.pointerId);
          setFromPointer(event);
        }}
        onPointerMove={(event) => {
          if (event.buttons === 0) return;
          setFromPointer(event);
        }}
      >
        <span className="vernier-track" aria-hidden="true" />
        {ticks.map((tick, index) => (
          <span
            key={`${tick.value}-${index}`}
            className={`vernier-tick${tick.major ? ' is-major' : ''}`}
            style={{ left: `${tick.ratio * 100}%` }}
            aria-hidden="true"
          />
        ))}
        <span className="vernier-thumb" style={{ left: `${ratio * 100}%` }} aria-hidden="true" />
      </div>

      <div className="vernier-scale" aria-hidden="true">
        <span>MIN {formatNumber(min, step, props.integer)}</span>
        <span>STEP {formatNumber(step, 0.001, false)}</span>
        <span>MAX {formatNumber(max, step, props.integer)}</span>
      </div>
    </div>
  );
}
